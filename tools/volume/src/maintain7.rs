// SPDX-License-Identifier: Apache-2.0
//! Explicit retention maintenance of an existing disposable v7 image.
//!
//! The host counterpart of the owner's `MAINTAIN_V7` job: one call to
//! `Volume7::maintain_retention`, which advances the retry epoch by exactly one,
//! drops every terminal retained record and frees the sectors only those
//! records' snapshots held. The owner answers `Busy` while an admission is
//! unresolved; so does this, before anything is written. It exists so host
//! fixtures can publish more files than the eight-record table holds (each
//! `add7` retains one record); it never evicts a record implicitly.
use std::path::Path;

use rustic_fs::Volume7;

use crate::add7::open_image;
use crate::command::hex;

pub(crate) fn maintain7(image: &Path) -> Result<String, String> {
    let mut disk = open_image(image)?;
    let mut volume = Volume7::EMPTY;
    volume
        .mount_into(&mut disk)
        .map_err(|error| format!("v7 mount refused: {error:?}"))?;
    let header = *volume
        .header()
        .map_err(|error| format!("v7 header unavailable: {error:?}"))?;
    let records = volume
        .retained_records()
        .map_err(|error| format!("v7 records unavailable: {error:?}"))?
        .iter()
        .flatten()
        .count();
    let free_before = volume
        .free_sectors()
        .map_err(|error| format!("v7 map unavailable: {error:?}"))?;
    let epoch = volume
        .maintain_retention(&mut disk)
        .map_err(|error| format!("v7 retention maintenance refused: {error:?}"))?;
    let sequence = volume
        .header()
        .map_err(|error| format!("v7 header unavailable: {error:?}"))?
        .sequence;
    let free = volume
        .free_sectors()
        .map_err(|error| format!("v7 map unavailable: {error:?}"))?;
    Ok(format!(
        "{{\"lineage\":\"{}\",\"sequence\":{sequence},\"previous_epoch\":{},\"epoch\":{epoch},\
         \"dropped\":{records},\"free_before\":{free_before},\"free_sectors\":{free}}}",
        hex(&header.lineage),
        header.epoch,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::add7::add7;
    use crate::disk::FileDisk;
    use crate::history5::seed5_history;
    use crate::migrate7::{migrate7, sha256_hex};
    use crate::testing::TempDir;
    use rustic_fs::WriteIdentity7;
    use std::path::PathBuf;

    const LINEAGE: &str = "4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d";

    fn digest(path: &Path) -> String {
        sha256_hex(&std::fs::read(path).unwrap())
    }

    fn input(dir: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// A v7 image migrated from the `receipts` history: workspace 5 with two
    /// retained direct commits, the same fixture the `add7` tests use.
    fn seeded(dir: &TempDir) -> PathBuf {
        let source = dir.path().join("receipts.v5");
        let image = dir.path().join("receipts.v7");
        seed5_history(&source, LINEAGE, "receipts").unwrap();
        migrate7(&source, &image, LINEAGE).unwrap();
        image
    }

    /// The unsigned integer after `"field":` in a one-line JSON answer.
    fn field(answer: &str, name: &str) -> u64 {
        let tail = answer.split(&format!("\"{name}\":")).nth(1).unwrap();
        tail.split([',', '}']).next().unwrap().parse().unwrap()
    }

    fn mounted(image: &Path) -> (FileDisk, Box<Volume7>) {
        let mut disk = FileDisk::open(image).unwrap();
        let mut volume = Box::new(Volume7::EMPTY);
        volume.mount_into(&mut disk).unwrap();
        (disk, volume)
    }

    #[test]
    fn records_drop_and_the_epoch_advances_so_add7_can_continue() {
        let dir = TempDir::new();
        let image = seeded(&dir);
        let source = input(&dir, "one.bin", &[9; 700]);
        for index in 0..6 {
            add7(&image, "5", &format!("f{index}.bin"), &source).unwrap();
        }
        let full = add7(&image, "5", "f6.bin", &source).unwrap_err();
        assert!(full.contains("never evicts"), "{full}");

        let before = digest(&image);
        let answer = maintain7(&image).unwrap();
        assert_ne!(digest(&image), before);
        let (_, volume) = mounted(&image);
        assert!(
            answer.starts_with(&format!("{{\"lineage\":\"{LINEAGE}\",")),
            "{answer}"
        );
        assert_eq!(
            field(&answer, "sequence"),
            volume.header().unwrap().sequence
        );
        assert_eq!(volume.header().unwrap().epoch, 2);
        assert_eq!(
            (field(&answer, "previous_epoch"), field(&answer, "epoch")),
            (1, 2)
        );
        assert_eq!(field(&answer, "dropped"), 8);
        assert_eq!(
            field(&answer, "free_sectors"),
            volume.free_sectors().unwrap()
        );
        assert!(field(&answer, "free_sectors") >= field(&answer, "free_before"));
        assert!(
            volume
                .retained_records()
                .unwrap()
                .iter()
                .all(Option::is_none)
        );
        drop(volume);
        let added = add7(&image, "5", "f6.bin", &source).unwrap();
        assert!(added.contains("\"epoch\":2"), "{added}");
    }

    #[test]
    fn snapshot_only_sectors_are_freed() {
        let dir = TempDir::new();
        let image = seeded(&dir);
        // Retire the migrated records first so only this test's two remain.
        maintain7(&image).unwrap();
        let source = input(&dir, "four.bin", &[3; 4 * 512]);
        add7(&image, "5", "four.bin", &source).unwrap();
        // Replace the file once more under a new key: its first payload is now
        // held only by the add7 record's snapshot.
        let before = {
            let (mut disk, mut volume) = mounted(&image);
            let node = volume.lookup(5, b"four.bin").unwrap();
            let identity = WriteIdentity7 {
                subject: 1,
                workspace: 5,
                object: node.id,
                instance: node.version,
                retry_epoch: 2,
                retry_key: 0x4000,
            };
            volume
                .replace_tracked(&mut disk, identity, node.version, &[4; 512])
                .unwrap();
            volume.free_sectors().unwrap()
        };
        let answer = maintain7(&image).unwrap();
        // The add7 record and the replacement's record.
        assert_eq!(field(&answer, "dropped"), 2);
        assert_eq!(field(&answer, "free_before"), before);
        assert_eq!(field(&answer, "free_sectors"), before + 4, "{answer}");
    }

    #[test]
    fn an_unresolved_admission_is_busy_and_the_image_unchanged() {
        let dir = TempDir::new();
        let source = dir.path().join("admissions.v5");
        let image = dir.path().join("admissions.v7");
        seed5_history(&source, LINEAGE, "admissions").unwrap();
        migrate7(&source, &image, LINEAGE).unwrap();
        let before = digest(&image);
        let error = maintain7(&image).unwrap_err();
        assert!(error.contains("Busy"), "{error}");
        assert_eq!(digest(&image), before);
    }

    #[test]
    fn a_missing_short_or_symlinked_image_is_refused() {
        let dir = TempDir::new();
        let image = seeded(&dir);
        assert!(maintain7(&dir.path().join("missing.v7")).is_err());
        let bytes = std::fs::read(&image).unwrap();
        let short = input(&dir, "short.v7", &bytes[..bytes.len() - 512]);
        let error = maintain7(&short).unwrap_err();
        assert!(error.contains("not an exact v7 image"), "{error}");
        let blank = input(&dir, "blank.v7", &vec![0; bytes.len()]);
        let error = maintain7(&blank).unwrap_err();
        assert!(error.contains("v7 mount refused"), "{error}");
        #[cfg(unix)]
        {
            let link = dir.path().join("link.v7");
            std::os::unix::fs::symlink(&image, &link).unwrap();
            let before = digest(&image);
            let error = maintain7(&link).unwrap_err();
            assert!(error.contains("symbolic link"), "{error}");
            assert_eq!(digest(&image), before);
        }
    }
}
