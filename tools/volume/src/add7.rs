// SPDX-License-Identifier: Apache-2.0
//! Add one host file to an existing disposable v7 image.
//!
//! This is a host publication step, not a migration and not a guest operation:
//! the image must already be an exact, mountable v7 image; the named directory
//! must lie inside `/workspaces`; the name must be new there. The file is
//! created and then filled by one tracked direct commit, so the retained record
//! names the bytes it published. Retention is never evicted: when no record
//! slot is free the command refuses before creating anything, as it does when
//! the free map cannot hold the file in at most eight runs. If the commit is
//! still refused after the create, a second publication removes the empty
//! node; if that publication is `Uncertain`, whether the node remains is
//! unknown.
use std::path::Path;
use std::str::FromStr;

use rustic_abi::files::reference::Workspace;
use rustic_fs::format7::{self, Node7};
use rustic_fs::{DATA_SECTORS, Kind, Volume7, WriteIdentity7};

use crate::command::{
    V7_IMAGE_BYTES, V7_IMAGE_SECTORS, hex, read_bounded, record_json, resource_text, valid_name,
    workspace_text,
};
use crate::disk::FileDisk;
use crate::migrate7::sha256_hex;

/// Root of every workspace the v7 file service will grant.
const WORKSPACES_ROOT: u32 = 4;
/// Retry subject of host publications, as `seed7` uses (the owner's subject).
const HOST_SUBJECT: u64 = 1;

pub(crate) fn add7(
    image: &Path,
    workspace: &str,
    name: &str,
    source: &Path,
) -> Result<String, String> {
    if !valid_name(name.as_bytes()) {
        return Err(
            "name must be 1..=31 ASCII letters, digits, '.', '_' or '-' and not . or ..".to_owned(),
        );
    }
    let bytes = read_bounded(source, "file", u64::from(format7::MAX_FILE_BYTES))?;
    if bytes.is_empty() {
        return Err(format!("file {} is empty", source.display()));
    }

    // `seed7` and `migrate7` refuse a symlinked target; so does this.
    let metadata = std::fs::symlink_metadata(image)
        .map_err(|error| format!("cannot open {}: {error}", image.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "{} is a symbolic link, not an image file",
            image.display()
        ));
    }
    let mut disk = FileDisk::open_regular(image)?;
    if disk.sectors() != V7_IMAGE_SECTORS || disk.bytes()? != V7_IMAGE_BYTES {
        return Err(format!(
            "{} is not an exact v7 image of {V7_IMAGE_BYTES} bytes",
            image.display()
        ));
    }
    let mut volume = Volume7::EMPTY;
    volume
        .mount_into(&mut disk)
        .map_err(|error| format!("v7 mount refused: {error:?}"))?;
    let lineage = volume
        .header()
        .map_err(|error| format!("v7 header unavailable: {error:?}"))?
        .lineage;
    let directory = resolve_workspace(&volume, lineage, workspace)?;

    // Every refusal below happens before the image is written.
    match volume.lookup(directory.id, name.as_bytes()) {
        Ok(_) => {
            return Err(format!(
                "{name} already exists in workspace {}",
                directory.id
            ));
        }
        Err(rustic_fs::Error::NotFound) => (),
        Err(error) => return Err(format!("v7 lookup refused: {error:?}")),
    }
    let records = volume
        .retained_records()
        .map_err(|error| format!("v7 records unavailable: {error:?}"))?;
    if records.iter().all(Option::is_some) {
        return Err("every retained record slot is held; add7 never evicts a record".to_owned());
    }
    let needed = (bytes.len() as u64).div_ceil(format7::SECTOR_BYTES);
    let free = volume
        .free_sectors()
        .map_err(|error| format!("v7 map unavailable: {error:?}"))?;
    if needed > free {
        return Err(format!(
            "file needs {needed} sectors; the image has {free} free"
        ));
    }
    let map = volume
        .allocation_map()
        .map_err(|error| format!("v7 map unavailable: {error:?}"))?;
    let reachable = largest_runs(map);
    if needed > reachable {
        return Err(format!(
            "file needs {needed} sectors; the free map holds only {reachable} in {} runs",
            format7::MAX_EXTENTS
        ));
    }
    let header = *volume
        .header()
        .map_err(|error| format!("v7 header unavailable: {error:?}"))?;
    let epoch = header.epoch;
    // The key search starts at the identity `create` will assign (the
    // header's next); any unused key is valid, and none is written yet.
    let retry_key = unused_key(&volume, directory.id, epoch, u64::from(header.next))?;

    let node = volume
        .create(&mut disk, directory.id, name.as_bytes(), Kind::File)
        .map_err(|error| format!("v7 file creation refused: {error:?}"))?;
    let identity = WriteIdentity7 {
        subject: HOST_SUBJECT,
        workspace: directory.id,
        object: node.id,
        instance: node.version,
        retry_epoch: epoch,
        retry_key,
    };
    let record = match volume.replace_tracked(&mut disk, identity, node.version, &bytes) {
        Ok(record) => record,
        Err(error) => {
            let cleanup = match volume.remove(&mut disk, node.id) {
                Ok(()) => "the empty file was removed".to_owned(),
                Err(removal) => format!("the empty file {} remains: {removal:?}", node.id),
            };
            return Err(format!("v7 write refused: {error:?}; {cleanup}"));
        }
    };
    let slot = volume
        .retained_records()
        .map_err(|error| format!("v7 records unavailable: {error:?}"))?
        .iter()
        .position(|held| *held == Some(record))
        .ok_or("the published record is not retained")?;
    let sequence = volume
        .header()
        .map_err(|error| format!("v7 header unavailable: {error:?}"))?
        .sequence;
    Ok(format!(
        "{{\"lineage\":\"{}\",\"sequence\":{sequence},\
         \"workspace\":{{\"id\":{},\"text\":\"{}\"}},\
         \"file\":{{\"id\":{},\"name\":\"{name}\",\"created\":{},\"version\":{},\"size\":{},\
         \"sha256\":\"{}\",\"resource\":\"{}\"}},\"record\":{}}}",
        hex(&lineage),
        directory.id,
        workspace_text(lineage, directory.id)?,
        node.id,
        node.version,
        record.committed,
        bytes.len(),
        sha256_hex(&bytes),
        resource_text(lineage, directory.id, node.id)?,
        record_json(slot, &record),
    ))
}

/// Resolve `text` (a node id, `ws_` text of this lineage, or an absolute path
/// such as `/workspaces/migrated`) to a directory inside `/workspaces`.
fn resolve_workspace(volume: &Volume7, lineage: [u8; 16], text: &str) -> Result<Node7, String> {
    let id = if let Some(path) = text.strip_prefix('/') {
        let mut parent = 0;
        for part in path.split('/') {
            parent = volume
                .lookup(parent, part.as_bytes())
                .map_err(|error| format!("workspace path {text}: {error:?}"))?
                .id;
        }
        parent
    } else if text.starts_with("ws_") {
        let named = Workspace::from_str(text)
            .map_err(|error| format!("invalid workspace text {text}: {error:?}"))?;
        if named.lineage() != lineage {
            return Err(format!("workspace {text} names another lineage"));
        }
        named.root()
    } else {
        text.parse()
            .map_err(|_| format!("workspace must be a node id, ws_ text or /path: {text}"))?
    };
    let node = volume
        .stat(id)
        .map_err(|error| format!("no workspace {text}: {error:?}"))?;
    if node.kind != Kind::Directory {
        return Err(format!("workspace {text} is not a directory"));
    }
    let mut current = node;
    for _ in 0..format7::NODES {
        if current.id == WORKSPACES_ROOT {
            return Ok(node);
        }
        if current.parent == 0 {
            break;
        }
        current = volume
            .stat(current.parent)
            .map_err(|error| format!("workspace {text} ancestry: {error:?}"))?;
    }
    Err(format!("workspace {text} is not inside /workspaces"))
}

/// Sectors the eight largest maximal free runs hold. `Volume7` plans a payload
/// by taking the largest free run not yet chosen, at most eight times, so a
/// file fits exactly when it needs no more than this.
fn largest_runs(map: &[u64]) -> u64 {
    let free = |sector: u64| map[sector as usize / 64] & (1u64 << (sector % 64)) == 0;
    let mut largest = [0u64; format7::MAX_EXTENTS];
    let mut sector = 0;
    while sector < DATA_SECTORS {
        let start = sector;
        while sector < DATA_SECTORS && free(sector) {
            sector += 1;
        }
        let run = sector - start;
        if let Some(smallest) = largest.iter_mut().min()
            && run > *smallest
        {
            *smallest = run;
        }
        sector += 1;
    }
    largest.iter().sum()
}

/// The first retry key from `start` that no retained record of the host
/// subject holds in this workspace and epoch, so a fresh commit is never read
/// as a retry of an existing record.
fn unused_key(volume: &Volume7, workspace: u32, epoch: u64, start: u64) -> Result<u64, String> {
    let records = volume
        .retained_records()
        .map_err(|error| format!("v7 records unavailable: {error:?}"))?;
    let held = |key: u64| {
        records.iter().flatten().any(|record| {
            record.subject == HOST_SUBJECT
                && record.workspace == workspace
                && record.retry_epoch == epoch
                && record.retry_key == key
        })
    };
    (start..start.saturating_add(format7::RETAINED as u64 + 1))
        .find(|key| !held(*key))
        .ok_or_else(|| "no unused retry key".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{parse_lineage, report7};
    use crate::history5::seed5_history;
    use crate::migrate7::migrate7;
    use crate::testing::TempDir;
    use rustic_fs::format7::Record7;
    use std::path::PathBuf;

    const LINEAGE: &str = "7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c";
    const MIGRATED: u32 = 5;

    /// A v7 image migrated from the `receipts` history: workspace 5
    /// (`/workspaces/migrated`) holding `direct.bin` and `owner.bin` with two
    /// retained direct commits.
    fn migrated(dir: &TempDir) -> PathBuf {
        let source = dir.path().join("receipts.v5");
        let target = dir.path().join("receipts.v7");
        seed5_history(&source, LINEAGE, "receipts").unwrap();
        migrate7(&source, &target, LINEAGE).unwrap();
        target
    }

    fn input(dir: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn pattern(seed: u8, length: usize) -> Vec<u8> {
        (0..length)
            .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    fn mounted(image: &Path) -> (FileDisk, Box<Volume7>) {
        let mut disk = FileDisk::open_read_only(image).unwrap();
        let mut volume = Box::new(Volume7::EMPTY);
        volume.mount_into(&mut disk).unwrap();
        (disk, volume)
    }

    fn records(image: &Path) -> Vec<Record7> {
        mounted(image)
            .1
            .retained_records()
            .unwrap()
            .iter()
            .flatten()
            .copied()
            .collect()
    }

    fn digest(path: &Path) -> String {
        sha256_hex(&std::fs::read(path).unwrap())
    }

    /// A refusal names `expected` and leaves the image byte-identical.
    fn assert_refused(image: &Path, workspace: &str, name: &str, source: &Path, expected: &str) {
        let before = digest(image);
        let error = add7(image, workspace, name, source).unwrap_err();
        assert!(
            error.contains(expected),
            "{error:?} does not name {expected}"
        );
        assert_eq!(digest(image), before, "a refused add7 changed the image");
    }

    #[test]
    fn two_pairs_are_added_beside_migrated_history_without_touching_it() {
        let dir = TempDir::new();
        let image = migrated(&dir);
        let migrated_records = records(&image);
        assert_eq!(migrated_records.len(), 2);
        let lineage = parse_lineage(LINEAGE).unwrap();

        let files = [
            (
                "tag-2.elf",
                pattern(2, 131_360),
                "/workspaces/migrated".to_owned(),
            ),
            ("tag-2.manifest", pattern(3, 128), MIGRATED.to_string()),
            (
                "tag-1.elf",
                pattern(1, 131_104),
                workspace_text(lineage, MIGRATED).unwrap(),
            ),
            ("tag-1.manifest", pattern(4, 128), MIGRATED.to_string()),
        ];
        let mut added = Vec::new();
        for (name, bytes, workspace) in &files {
            let source = input(&dir, &format!("source-{name}"), bytes);
            let output = add7(&image, workspace, name, &source).unwrap();
            added.push((output, bytes));
        }

        let (mut disk, volume) = mounted(&image);
        let header = *volume.header().unwrap();
        assert!(!volume.recovered_from_header().unwrap());
        for (index, (output, bytes)) in added.iter().enumerate() {
            // Objects 6 and 7 came from the source; each add is one create and
            // one commit, so ids start at 8 and versions advance by two.
            let id = 8 + index as u32;
            let created = 13 + 2 * index as u64;
            let resource = resource_text(lineage, MIGRATED, id).unwrap();
            assert!(
                output.starts_with(&format!(
                    "{{\"lineage\":\"{LINEAGE}\",\"sequence\":{},\
                 \"workspace\":{{\"id\":5,\"text\":\"ws_{LINEAGE}_00000005\"}},\
                 \"file\":{{\"id\":{id},\"name\":\"{}\",\"created\":{created},\"version\":{},\
                 \"size\":{},\"sha256\":\"{}\",\"resource\":\"{resource}\"}},\"record\":{{",
                    created + 1,
                    files[index].0,
                    created + 1,
                    bytes.len(),
                    sha256_hex(bytes),
                )),
                "{output}"
            );
            assert!(output.contains(&format!(
                "\"state\":\"direct_committed\",\"cause\":null,\"subject\":1,\"workspace\":5,\
                 \"object\":{id},\"instance\":{created},\"epoch\":1,\"key\":{id},\
                 \"previous\":{created},\"committed\":{}",
                created + 1
            )));
            let node = volume.stat(id).unwrap();
            assert_eq!(
                (node.parent, node.version, node.length as usize),
                (MIGRATED, created + 1, bytes.len())
            );
            let mut read = vec![0; bytes.len()];
            let count = volume
                .read_range(&mut disk, id, Some(created + 1), 0, &mut read)
                .unwrap();
            assert_eq!((count, &read), (bytes.len(), *bytes));
        }
        assert_eq!(header.sequence, 20);
        let after = records(&image);
        assert_eq!(after.len(), 6);
        for record in &migrated_records {
            assert!(
                after.contains(record),
                "a migrated record changed: {record:?}"
            );
        }
        assert!(report7(&image).unwrap().contains("\"recovered\":false"));
    }

    #[test]
    fn a_full_retention_table_is_refused_before_anything_is_created() {
        let dir = TempDir::new();
        let image = migrated(&dir);
        let source = input(&dir, "one.bin", &pattern(9, 700));
        for index in 0..(format7::RETAINED - 2) {
            add7(&image, "5", &format!("f{index}.bin"), &source).unwrap();
        }
        assert_eq!(records(&image).len(), format7::RETAINED);
        assert_refused(&image, "5", "last.bin", &source, "never evicts");
        assert!(mounted(&image).1.lookup(MIGRATED, b"last.bin").is_err());
    }

    #[test]
    fn missing_foreign_and_non_workspace_directories_are_refused() {
        let dir = TempDir::new();
        let image = migrated(&dir);
        let source = input(&dir, "one.bin", &pattern(9, 700));
        let foreign = format!("ws_{}_00000005", "23".repeat(16));
        for (workspace, expected) in [
            ("99", "no workspace"),
            ("6", "not a directory"),
            ("1", "not inside /workspaces"),
            ("/workspaces/nowhere", "NotFound"),
            ("/workspaces/", "workspace path"),
            (foreign.as_str(), "another lineage"),
            ("ws_nonsense", "invalid workspace text"),
            ("migrated", "must be a node id"),
        ] {
            assert_refused(&image, workspace, "new.bin", &source, expected);
        }
    }

    #[test]
    fn an_existing_name_invalid_name_empty_or_oversized_file_is_refused() {
        let dir = TempDir::new();
        let image = migrated(&dir);
        let source = input(&dir, "one.bin", &pattern(9, 700));
        assert_refused(&image, "5", "direct.bin", &source, "already exists");
        for name in ["", "..", "a/b", "has space", &"n".repeat(32)] {
            assert_refused(&image, "5", name, &source, "name must be");
        }
        let empty = input(&dir, "empty.bin", b"");
        assert_refused(&image, "5", "empty.bin", &empty, "is empty");
        let oversized = input(
            &dir,
            "oversized.bin",
            &vec![1; format7::MAX_FILE_BYTES as usize + 1],
        );
        assert_refused(&image, "5", "big.bin", &oversized, "maximum");
        assert_refused(
            &image,
            "5",
            "gone.bin",
            &dir.path().join("missing"),
            "cannot read",
        );
    }

    /// Commit `length` pattern bytes into a new file, first retiring terminal
    /// records when the table is full so the fixture is not bounded by it.
    fn fill(volume: &mut Volume7, disk: &mut FileDisk, name: &str, length: usize) -> u32 {
        if volume
            .retained_records()
            .unwrap()
            .iter()
            .all(Option::is_some)
        {
            volume.maintain_retention(disk).unwrap();
        }
        let node = volume.create(disk, 5, name.as_bytes(), Kind::File).unwrap();
        let identity = WriteIdentity7 {
            subject: HOST_SUBJECT,
            workspace: 5,
            object: node.id,
            instance: node.version,
            retry_epoch: volume.header().unwrap().epoch,
            retry_key: u64::from(node.id),
        };
        volume
            .replace_tracked(disk, identity, node.version, &pattern(7, length))
            .unwrap();
        node.id
    }

    /// A fresh image whose only free payload is nine one-sector holes.
    fn fragmented(dir: &TempDir) -> PathBuf {
        let image = dir.path().join("fragmented.v7");
        let mut disk = FileDisk::create_new(&image, V7_IMAGE_SECTORS).unwrap();
        let mut volume = Box::new(Volume7::EMPTY);
        volume
            .provision_into(&mut disk, parse_lineage(LINEAGE).unwrap())
            .unwrap();
        volume
            .create(&mut disk, 4, b"work", Kind::Directory)
            .unwrap();
        // The planner takes the largest free run, so on a fresh map each file
        // lands directly after the previous one.
        let small: Vec<u32> = (0..18)
            .map(|index| fill(&mut volume, &mut disk, &format!("s{index}"), 512))
            .collect();
        let mut index = 0;
        loop {
            let free = volume.free_sectors().unwrap();
            if free == 0 {
                break;
            }
            let length = free.min(u64::from(format7::MAX_FILE_BYTES) / 512) as usize * 512;
            fill(&mut volume, &mut disk, &format!("b{index}"), length);
            index += 1;
        }
        volume.maintain_retention(&mut disk).unwrap();
        for id in small.iter().step_by(2) {
            volume.remove(&mut disk, *id).unwrap();
        }
        assert_eq!(volume.free_sectors().unwrap(), 9);
        assert_eq!(largest_runs(volume.allocation_map().unwrap()), 8);
        image
    }

    #[test]
    fn a_file_the_fragmented_map_cannot_hold_in_eight_runs_is_refused_before_writing() {
        let dir = TempDir::new();
        let image = fragmented(&dir);
        // Nine sectors are free, but only as nine runs; the raw count passes.
        let nine = input(&dir, "nine.bin", &pattern(5, 9 * 512));
        assert_refused(&image, "5", "nine.bin", &nine, "only 8 in 8 runs");
        assert!(mounted(&image).1.lookup(5, b"nine.bin").is_err());
        // Eight sectors fit in eight runs and are published.
        let eight = input(&dir, "eight.bin", &pattern(6, 8 * 512));
        add7(&image, "/workspaces/work", "eight.bin", &eight).unwrap();
        let (mut disk, volume) = mounted(&image);
        let node = volume.lookup(5, b"eight.bin").unwrap();
        let mut read = vec![0; 8 * 512];
        volume
            .read_range(&mut disk, node.id, Some(node.version), 0, &mut read)
            .unwrap();
        assert_eq!(read, pattern(6, 8 * 512));
        assert_eq!(volume.free_sectors().unwrap(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_image_path_is_refused_and_its_target_left_unchanged() {
        let dir = TempDir::new();
        let image = migrated(&dir);
        let source = input(&dir, "one.bin", &pattern(9, 700));
        let link = dir.path().join("link.v7");
        std::os::unix::fs::symlink(&image, &link).unwrap();
        assert_refused(&link, "5", "new.bin", &source, "symbolic link");
        assert_eq!(records(&image).len(), 2);
    }

    #[test]
    fn an_image_that_is_not_exactly_one_v7_volume_is_refused() {
        let dir = TempDir::new();
        let image = migrated(&dir);
        let source = input(&dir, "one.bin", &pattern(9, 700));
        let bytes = std::fs::read(&image).unwrap();

        let short = input(&dir, "short.v7", &bytes[..bytes.len() - 512]);
        assert_refused(&short, "5", "new.bin", &source, "not an exact v7 image");
        let mut longer = bytes.clone();
        longer.extend_from_slice(&[0; 512]);
        let long = input(&dir, "long.v7", &longer);
        assert_refused(&long, "5", "new.bin", &source, "not an exact v7 image");
        let v5 = dir.path().join("receipts.v5");
        assert_refused(&v5, "5", "new.bin", &source, "not an exact v7 image");
        let blank = input(&dir, "blank.v7", &vec![0; bytes.len()]);
        assert_refused(&blank, "5", "new.bin", &source, "v7 mount refused");
        let directory = dir.path().join("directory.v7");
        std::fs::create_dir(&directory).unwrap();
        assert!(add7(&directory, "5", "new.bin", &source).is_err());
    }
}
