// SPDX-License-Identifier: Apache-2.0
//! Deliberate out-of-place v5 -> v7 data migration of a disposable image.
//!
//! The source is opened read-only and must be exactly one legacy tool image
//! long; its SHA-256 is taken before and after, and the two must agree. The
//! target is created exclusively, so an existing file (including the source)
//! is never overwritten, and it is removed again when anything after its
//! creation fails, including the verifying remount. This migrates data only:
//! it neither boots nor rolls back any executable.
use std::path::Path;

use rustic_fs::{Volume7, format7, upgrade_v5_to_v7};
use sha2::{Digest, Sha256};

use crate::command::{IMAGE_SECTORS, parse_lineage, report7};
use crate::disk::FileDisk;

/// The only accepted source size: what `seed` and `seed5-history` create.
const SOURCE_BYTES: u64 = IMAGE_SECTORS * 512;

pub(crate) fn migrate7(source: &Path, target: &Path, lineage: &str) -> Result<String, String> {
    let lineage = parse_lineage(lineage)?;
    let mut source_disk = FileDisk::open_read_only(source)?;
    if source_disk.sectors() != IMAGE_SECTORS || source_disk.bytes()? != SOURCE_BYTES {
        return Err(format!(
            "{} is not an exact v5 image of {SOURCE_BYTES} bytes",
            source.display()
        ));
    }
    let before = sha256_disk(&mut source_disk)?;

    let mut target_disk = FileDisk::create_new(target, format7::VOLUME_SECTORS)?;
    let created = created_identity(&target_disk);
    // Hold the created file open until cleanup: while it is open its inode
    // cannot be reused, so an identity match can only name this file.
    let _held = std::fs::File::open(target)
        .map_err(|error| format!("cannot hold {}: {error}", target.display()))?;
    let result = (|| {
        let mut volume = Volume7::EMPTY;
        upgrade_v5_to_v7(&mut source_disk, &mut target_disk, &mut volume, lineage)
            .map_err(|error| format!("migration refused: {error:?}"))?;
        drop(target_disk);
        let report = report7(target)?;
        let after = sha256_disk(&mut source_disk)?;
        if after != before {
            return Err("the source image changed during migration".to_owned());
        }
        Ok(format!(
            "{{\"source_bytes\":{SOURCE_BYTES},\"source_sha256_before\":\"{before}\",\
             \"source_sha256_after\":\"{after}\",\"target\":{report}}}"
        ))
    })();
    result.map_err(|error| match remove_created(target, created) {
        Ok(()) => error,
        Err(kept) => format!("{error}; {kept}"),
    })
}

/// The created target's identity, or `None` where it cannot be established.
fn created_identity(disk: &FileDisk) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        disk.identity().ok()
    }
    #[cfg(not(unix))]
    {
        let _ = disk;
        None
    }
}

/// Remove the target only while its path still names the regular file this
/// invocation created; anything else found there is left alone and reported.
fn remove_created(target: &Path, created: Option<(u64, u64)>) -> Result<(), String> {
    let kept = || {
        format!(
            "{} was left in place: it no longer names the target this invocation created",
            target.display()
        )
    };
    let Some(created) = created else {
        return Err(kept());
    };
    let Ok(metadata) = std::fs::symlink_metadata(target) else {
        return Err(kept());
    };
    #[cfg(unix)]
    let same = {
        use std::os::unix::fs::MetadataExt;
        metadata.file_type().is_file() && (metadata.dev(), metadata.ino()) == created
    };
    #[cfg(not(unix))]
    let same = {
        let _ = (metadata, created);
        false
    };
    if !same {
        return Err(kept());
    }
    std::fs::remove_file(target)
        .map_err(|error| format!("cannot remove {}: {error}", target.display()))
}

/// SHA-256 of an exact-size source, streamed through its open handle.
fn sha256_disk(disk: &mut FileDisk) -> Result<String, String> {
    let mut digest = Sha256::new();
    let total = disk.stream(|bytes| digest.update(bytes))?;
    if total != SOURCE_BYTES {
        return Err("the source image changed size while being read".to_owned());
    }
    Ok(hex_digest(digest.finalize().into()))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex_digest(Sha256::digest(bytes).into())
}

fn hex_digest(digest: [u8; 32]) -> String {
    crate::command::hex(&digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history5::{OWNER_SUBJECT, SHELL_SUBJECT, pattern, seed5_history};
    use crate::testing::TempDir;
    use rustic_fs::format7::RecordState;
    use rustic_fs::{Kind, PreventionReason, Replacement, Retry, Volume};
    use std::io::{Seek, SeekFrom, Write};
    use std::path::PathBuf;

    const LINEAGE: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";
    const FOREIGN: &str = "23232323232323232323232323232323";

    fn digest_of(path: &Path) -> String {
        sha256_disk(&mut FileDisk::open_read_only(path).unwrap()).unwrap()
    }

    fn lineage_bytes(text: &str) -> [u8; 16] {
        parse_lineage(text).unwrap()
    }

    fn seeded(dir: &TempDir, set: &str) -> PathBuf {
        let source = dir.path().join(format!("{set}.v5"));
        seed5_history(&source, LINEAGE, set).unwrap();
        source
    }

    /// A refusal leaves the source byte-identical and no target behind.
    fn assert_refused(source: &Path, target: &Path, lineage: &str, expected: &str) {
        let before = std::fs::read(source).ok();
        let error = migrate7(source, target, lineage).unwrap_err();
        assert!(
            error.contains(expected),
            "{error:?} does not name {expected}"
        );
        assert!(!target.exists(), "a refused migration left its target");
        assert_eq!(std::fs::read(source).ok(), before, "the source changed");
    }

    fn patch(path: &Path, sector: u64, bytes: &[u8; 512]) {
        let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.seek(SeekFrom::Start(sector * 512)).unwrap();
        file.write_all(bytes).unwrap();
    }

    fn sector(path: &Path, sector: u64) -> [u8; 512] {
        let mut disk = FileDisk::open_read_only(path).unwrap();
        let mut bytes = [0; 512];
        rustic_fs::Disk::read(&mut disk, sector, &mut bytes).unwrap();
        bytes
    }

    /// The selected v5 metadata bank's header sector and node table.
    fn selected_table(path: &Path) -> (u64, [u8; 512], [u8; 2048]) {
        let left = sector(path, 8);
        let right = sector(path, 13);
        let sequence = |header: &[u8; 512]| u64::from_le_bytes(header[12..20].try_into().unwrap());
        let header_sector = if sequence(&right) > sequence(&left) {
            13
        } else {
            8
        };
        let mut table = [0; 2048];
        for (index, block) in table.as_chunks_mut::<512>().0.iter_mut().enumerate() {
            *block = sector(path, header_sector + 1 + index as u64);
        }
        (header_sector, sector(path, header_sector), table)
    }

    fn slot_of(table: &[u8; 2048], id: u32) -> usize {
        (0..32)
            .find(|slot| {
                let node = &table[slot * 64..slot * 64 + 64];
                node[0] != 0 && u32::from_le_bytes(node[8..12].try_into().unwrap()) == id
            })
            .unwrap()
    }

    #[test]
    fn every_history_set_migrates_with_an_unchanged_source_and_exact_snapshots() {
        let dir = TempDir::new();
        let lineage = lineage_bytes(LINEAGE);
        let expected = [
            (
                "receipts",
                vec![
                    (RecordState::DirectCommitted, SHELL_SUBJECT, None, 5, 700),
                    (RecordState::DirectCommitted, OWNER_SUBJECT, None, 6, 300),
                ],
            ),
            (
                "admissions",
                vec![
                    (RecordState::Admitted, SHELL_SUBJECT, None, 7, 900),
                    (
                        RecordState::Cancelled,
                        SHELL_SUBJECT,
                        Some(PreventionReason::Requested),
                        8,
                        400,
                    ),
                ],
            ),
            (
                "completed",
                vec![(RecordState::AdmittedCommitted, SHELL_SUBJECT, None, 9, 1000)],
            ),
        ];
        for (set, records) in expected {
            let source = seeded(&dir, set);
            let before = digest_of(&source);
            let target = dir.path().join(format!("{set}.v7"));
            let output = migrate7(&source, &target, LINEAGE).unwrap();
            assert!(output.contains(&format!("\"source_sha256_before\":\"{before}\"")));
            assert!(output.contains(&format!("\"source_sha256_after\":\"{before}\"")));
            assert_eq!(digest_of(&source), before);
            assert!(output.contains(&format!("\"target\":{{\"lineage\":\"{LINEAGE}\"")));

            let mut disk = FileDisk::open_read_only(&target).unwrap();
            let mut volume = Volume7::EMPTY;
            volume.mount_into(&mut disk).unwrap();
            let header = *volume.header().unwrap();
            assert_eq!((header.lineage, header.epoch), (lineage, 1));
            let migrated: Vec<_> = volume
                .retained_records()
                .unwrap()
                .iter()
                .flatten()
                .copied()
                .collect();
            assert_eq!(migrated.len(), records.len(), "{set}");
            for (record, (state, subject, cause, seed, size)) in migrated.iter().zip(records) {
                assert_eq!(
                    (
                        record.state,
                        record.subject,
                        record.prevention,
                        record.workspace
                    ),
                    (state, subject, cause, 5),
                    "{set}"
                );
                let mut snapshot = vec![0; size];
                let mut offset = 0;
                while offset < size {
                    let mut block = [0; 512];
                    let read = volume
                        .read_retained_range(&mut disk, record, offset as u64, &mut block)
                        .unwrap();
                    snapshot[offset..offset + read].copy_from_slice(&block[..read]);
                    offset += read;
                }
                assert_eq!(snapshot, pattern(seed, size), "{set} snapshot bytes");
            }
        }
    }

    #[test]
    fn malformed_and_all_zero_lineages_are_refused_before_any_target_exists() {
        let dir = TempDir::new();
        let source = seeded(&dir, "receipts");
        let target = dir.path().join("target.v7");
        for lineage in ["5a5a", "zz5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a", &"0".repeat(32)] {
            assert_refused(&source, &target, lineage, "lineage");
        }
    }

    #[test]
    fn a_foreign_envelope_lineage_is_refused_and_the_target_removed() {
        let dir = TempDir::new();
        let source = seeded(&dir, "receipts");
        assert_refused(&source, &dir.path().join("t.v7"), FOREIGN, "Lineage");
    }

    #[test]
    fn a_foreign_recovery_lineage_without_an_envelope_is_refused() {
        let dir = TempDir::new();
        let source = dir.path().join("recovery.v5");
        let mut disk = FileDisk::create_new(&source, IMAGE_SECTORS).unwrap();
        let mut volume = Volume::initialize(&mut disk).unwrap();
        volume.create(&mut disk, 4, b"file", Kind::File).unwrap();
        volume
            .enable_recovery(&mut disk, lineage_bytes(LINEAGE))
            .unwrap();
        drop(disk);
        assert_eq!(sector(&source, 1), [0; 512], "no envelope names a lineage");
        assert_refused(&source, &dir.path().join("t.v7"), FOREIGN, "Lineage");
    }

    #[test]
    fn a_scope_less_retained_record_is_refused_as_unsupported() {
        let dir = TempDir::new();
        let source = dir.path().join("legacy.v5");
        let mut disk = FileDisk::create_new(&source, IMAGE_SECTORS).unwrap();
        let mut volume = Volume::initialize(&mut disk).unwrap();
        let file = volume.create(&mut disk, 4, b"file", Kind::File).unwrap();
        volume
            .enable_recovery(&mut disk, lineage_bytes(LINEAGE))
            .unwrap();
        let retry = Retry {
            lineage: lineage_bytes(LINEAGE),
            epoch: 1,
            key: 7,
        };
        volume
            .replace_tracked(
                &mut disk,
                1,
                retry,
                file.id,
                file.version,
                b"legacy receipt",
            )
            .unwrap();
        drop(disk);
        assert_refused(&source, &dir.path().join("t.v7"), LINEAGE, "Unsupported");
    }

    #[test]
    fn a_corrupt_live_payload_is_refused() {
        let dir = TempDir::new();
        let source = seeded(&dir, "receipts");
        let (_, _, table) = selected_table(&source);
        let slot = slot_of(&table, 6);
        let data_sector = 32 + slot as u64 * 4 + u64::from(table[slot * 64 + 2]) * 2;
        let mut payload = sector(&source, data_sector);
        payload[3] ^= 0x40;
        patch(&source, data_sector, &payload);
        assert_refused(&source, &dir.path().join("t.v7"), LINEAGE, "Corrupt");
    }

    #[test]
    fn history_the_v7_validator_cannot_represent_is_refused() {
        // A checksum-valid live version below its own committed receipt: v5
        // metadata that v7's whole-generation history rules must refuse.
        let dir = TempDir::new();
        let source = seeded(&dir, "receipts");
        let (header_sector, mut header, mut table) = selected_table(&source);
        let node = slot_of(&table, 6) * 64;
        table[node + 16..node + 24].copy_from_slice(&1u64.to_le_bytes());
        header[24..28].copy_from_slice(&format7::aggregate(&table).to_le_bytes());
        header[28..32].fill(0);
        let checksum = format7::aggregate(&header);
        header[28..32].copy_from_slice(&checksum.to_le_bytes());
        for (index, block) in table.as_chunks::<512>().0.iter().enumerate() {
            patch(&source, header_sector + 1 + index as u64, block);
        }
        patch(&source, header_sector, &header);
        assert_refused(&source, &dir.path().join("t.v7"), LINEAGE, "Corrupt");
    }

    #[test]
    fn a_blank_source_is_refused_as_empty() {
        let dir = TempDir::new();
        let source = dir.path().join("blank.v5");
        drop(FileDisk::create_new(&source, IMAGE_SECTORS).unwrap());
        assert_refused(&source, &dir.path().join("t.v7"), LINEAGE, "Empty");
    }

    #[test]
    fn a_source_of_any_other_size_is_refused_before_the_target_exists() {
        let dir = TempDir::new();
        let source = seeded(&dir, "receipts");
        let target = dir.path().join("t.v7");
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&source)
            .unwrap();
        file.set_len(SOURCE_BYTES - 512).unwrap();
        assert_refused(&source, &target, LINEAGE, "exact v5 image");
        file.set_len(SOURCE_BYTES + 512).unwrap();
        assert_refused(&source, &target, LINEAGE, "exact v5 image");
        let v7_sized = format7::VOLUME_SECTORS * 512;
        assert_ne!(v7_sized, SOURCE_BYTES);
        file.set_len(v7_sized).unwrap();
        assert_refused(&source, &target, LINEAGE, "exact v5 image");
    }

    #[test]
    fn an_existing_target_is_refused_and_left_untouched() {
        let dir = TempDir::new();
        let source = seeded(&dir, "receipts");
        let target = dir.path().join("existing.v7");
        std::fs::write(&target, b"preserve this file").unwrap();
        let before = std::fs::read(&source).unwrap();
        let error = migrate7(&source, &target, LINEAGE).unwrap_err();
        assert!(error.contains("cannot create new"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), b"preserve this file");
        assert_eq!(std::fs::read(&source).unwrap(), before);
        // The source itself is an existing path, so it can never be the target.
        let error = migrate7(&source, &source, LINEAGE).unwrap_err();
        assert!(error.contains("cannot create new"), "{error}");
        assert_eq!(std::fs::read(&source).unwrap(), before);
    }

    #[test]
    fn seed5_history_refuses_an_existing_image_and_an_unknown_set() {
        let dir = TempDir::new();
        let image = dir.path().join("existing.v5");
        std::fs::write(&image, b"keep").unwrap();
        assert!(seed5_history(&image, LINEAGE, "receipts").is_err());
        assert_eq!(std::fs::read(&image).unwrap(), b"keep");
        let fresh = dir.path().join("fresh.v5");
        assert!(seed5_history(&fresh, LINEAGE, "everything").is_err());
        assert!(!fresh.exists());
    }

    #[test]
    fn an_operations_only_v5_source_migrates_its_scoped_direct_receipts_exactly() {
        // Scoped operations without the later admission or prevention-cause
        // capabilities: the oldest v5 record layout the converter carries.
        let dir = TempDir::new();
        let lineage = lineage_bytes(LINEAGE);
        let source = dir.path().join("operations.v5");
        let mut disk = FileDisk::create_new(&source, IMAGE_SECTORS).unwrap();
        let mut volume = Volume::initialize(&mut disk).unwrap();
        let workspace = volume
            .create(&mut disk, 4, b"ops", Kind::Directory)
            .unwrap();
        let older = volume
            .create(&mut disk, workspace.id, b"older", Kind::File)
            .unwrap();
        let current = volume
            .create(&mut disk, workspace.id, b"current", Kind::File)
            .unwrap();
        volume.enable_recovery(&mut disk, lineage).unwrap();
        volume.enable_operations(&mut disk).unwrap();
        let request = |file: &rustic_fs::Node, key: u64| Replacement {
            workspace: workspace.id,
            retry: Retry {
                lineage,
                epoch: 1,
                key,
            },
            id: file.id,
            version: file.version,
        };
        let first = volume
            .replace_scoped(
                &mut disk,
                SHELL_SUBJECT,
                0,
                request(&older, 0x40),
                &pattern(3, 600),
            )
            .unwrap();
        // The live file moves on, so this snapshot cannot alias live payload.
        volume
            .replace(&mut disk, older.id, first.committed, b"newer live bytes")
            .unwrap();
        let second = volume
            .replace_scoped(
                &mut disk,
                OWNER_SUBJECT,
                first.committed,
                request(&current, 0x41),
                &pattern(4, 1024),
            )
            .unwrap();
        drop(disk);

        let target = dir.path().join("operations.v7");
        migrate7(&source, &target, LINEAGE).unwrap();
        let mut disk = FileDisk::open_read_only(&target).unwrap();
        let mut migrated = Volume7::EMPTY;
        migrated.mount_into(&mut disk).unwrap();
        let records: Vec<_> = migrated
            .retained_records()
            .unwrap()
            .iter()
            .flatten()
            .copied()
            .collect();
        let expected = [
            (SHELL_SUBJECT, older.id, first, 600usize, 3u8),
            (OWNER_SUBJECT, current.id, second, 1024, 4),
        ];
        assert_eq!(records.len(), expected.len());
        for (record, (subject, object, receipt, size, seed)) in records.iter().zip(expected) {
            assert_eq!(
                (
                    record.state,
                    record.subject,
                    record.workspace,
                    record.object
                ),
                (RecordState::DirectCommitted, subject, workspace.id, object)
            );
            assert_eq!(
                (record.instance, record.retry_epoch, record.retry_key),
                (first.committed, receipt.retry.epoch, receipt.retry.key)
            );
            assert_eq!(
                (
                    record.previous,
                    record.committed,
                    record.admission_number,
                    record.terminal,
                    record.length,
                    record.prevention,
                ),
                (
                    receipt.previous,
                    receipt.committed,
                    0,
                    receipt.committed,
                    size as u32,
                    None,
                )
            );
            let mut snapshot = vec![0; size];
            let mut offset = 0;
            while offset < size {
                let mut block = [0; 512];
                let read = migrated
                    .read_retained_range(&mut disk, record, offset as u64, &mut block)
                    .unwrap();
                snapshot[offset..offset + read].copy_from_slice(&block[..read]);
                offset += read;
            }
            assert_eq!(snapshot, pattern(seed, size));
        }
    }

    #[cfg(unix)]
    #[test]
    fn failure_cleanup_removes_only_the_file_this_invocation_created() {
        let dir = TempDir::new();
        let target = dir.path().join("created.v7");
        let created = created_identity(&FileDisk::create_new(&target, 1).unwrap());
        remove_created(&target, created).unwrap();
        assert!(!target.exists());

        // Replaced by another file after creation: left in place and reported.
        // The created file stays open, as in `migrate7`, so its inode cannot be
        // reused by the replacement.
        let held = FileDisk::create_new(&target, 1).unwrap();
        let created = created_identity(&held);
        std::fs::remove_file(&target).unwrap();
        std::fs::write(&target, b"someone else's file").unwrap();
        assert!(
            remove_created(&target, created)
                .unwrap_err()
                .contains("left in place")
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"someone else's file");
        drop(held);

        // Replaced by a symlink to the created file: the link is not removed.
        std::fs::remove_file(&target).unwrap();
        let moved = dir.path().join("moved.v7");
        let created = created_identity(&FileDisk::create_new(&moved, 1).unwrap());
        std::os::unix::fs::symlink(&moved, &target).unwrap();
        assert!(remove_created(&target, created).is_err());
        assert!(
            std::fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(moved.exists());
    }
}
