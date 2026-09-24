// SPDX-License-Identifier: Apache-2.0
//! Cold profile-2 lookups of retained records: the receipt is rebuilt from the
//! record and a SHA-256 streamed from its snapshot, within the caller's
//! subject and scope, with the v5 answers for missing records.
use super::*;
use rustic_abi::files::{OPERATION_ID, OPERATION_RETRY};

impl Writer {
    /// Look up one receipt and collect its remaining parts on this slot.
    fn lookup(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        query: operation::Lookup,
    ) -> Result<(Operation, [u8; RECEIPT_BYTES]), Error> {
        let first = self.send(server, disk, Lookup { query }.packet(self.context));
        status(first)?;
        let id = OperationId::new(LINEAGE, first.version).unwrap();
        let mut bytes = [0; RECEIPT_BYTES];
        for offset in [0usize, 40, 80] {
            let part = if offset == 0 {
                first
            } else {
                self.part(server, disk, id, offset)?
            };
            let length = (RECEIPT_BYTES - offset).min(DATA);
            assert_eq!(
                (part.id, part.arg, part.version, part.count as usize),
                (offset as u32, RECEIPT_BYTES as u32, id.sequence(), length)
            );
            bytes[offset..offset + length].copy_from_slice(part.payload());
        }
        let operation = Operation::decode(&bytes).unwrap();
        assert_eq!(operation.id, id);
        Ok((operation, bytes))
    }
}

fn by_id(receipt: &Operation) -> operation::Lookup {
    operation::Lookup::Id(receipt.id)
}

fn by_retry(receipt: &Operation) -> operation::Lookup {
    operation::Lookup::Retry {
        workspace: receipt.workspace,
        retry: receipt.retry,
    }
}

/// Commit a small and then a large write to the same file, so the small
/// write's snapshot is no longer the live content, and remount cold.
fn two_writes(f: &mut Fixture) -> [(Operation, [u8; RECEIPT_BYTES], Vec<u8>); 2] {
    let small = pattern(1, 513);
    let large = pattern(2, 300 * 1024 + 3);
    let mut server = Server7::new(&mut f.volume);
    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    let first = replacement(
        server.volume(),
        f.workspace,
        f.file,
        version(server.volume(), f.file),
        0x101,
    );
    let (a, a_bytes) = writer
        .replace(&mut server, &mut f.disk, first, &small)
        .unwrap();
    let second = replacement(
        server.volume(),
        f.workspace,
        f.file,
        a.version.value(),
        0x102,
    );
    let (b, b_bytes) = writer
        .replace(&mut server, &mut f.disk, second, &large)
        .unwrap();
    [(a, a_bytes, small), (b, b_bytes, large)]
}

fn remount(disk: &mut Sparse) -> Volume7 {
    let mut volume = Volume7::EMPTY;
    volume.mount_into(disk).unwrap();
    volume
}

#[test]
fn cold_lookups_by_id_and_retry_rebuild_the_commit_receipt_and_its_digest() {
    let mut f = fixture();
    let writes = two_writes(&mut f);
    let mut volume = remount(&mut f.disk);
    let mut server = Server7::new(&mut volume);
    // A different slot of the same subject, after a remount: nothing is cached.
    let reader = Writer::grant(&mut server, 3, f.workspace, TRACKED_WRITE7, SUBJECT);
    for (receipt, encoded, content) in &writes {
        for query in [by_id(receipt), by_retry(receipt)] {
            let (reads, disk_writes, flushes) = (f.disk.reads, f.disk.writes, f.disk.flushes);
            let (found, bytes) = reader.lookup(&mut server, &mut f.disk, query).unwrap();
            assert_eq!(&bytes, encoded, "byte-identical to the commit receipt");
            assert_eq!(found.sha256, <[u8; 32]>::from(Sha256::digest(content)));
            // One read per snapshot sector for the first part; the later parts
            // come from this slot's cached receipt. Nothing is written.
            assert_eq!(f.disk.reads - reads, content.len().div_ceil(512));
            assert_eq!((f.disk.writes, f.disk.flushes), (disk_writes, flushes));
        }
    }
    // The live file is the large write; the small snapshot kept its own bytes.
    drop(server);
    assert_eq!(read_all(&volume, &mut f.disk, f.file), writes[1].2);
}

#[test]
fn a_record_whose_file_was_removed_stays_visible_through_its_workspace() {
    let mut f = fixture();
    let writes = two_writes(&mut f);
    f.volume.remove(&mut f.disk, f.file).unwrap();
    let mut volume = remount(&mut f.disk);
    let mut server = Server7::new(&mut volume);
    let reader = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    let (receipt, encoded, _) = &writes[0];
    let (_, bytes) = reader
        .lookup(&mut server, &mut f.disk, by_retry(receipt))
        .unwrap();
    assert_eq!(&bytes, encoded);
}

#[test]
fn missing_records_answer_like_v5_lookups() {
    let mut f = fixture();
    let writes = two_writes(&mut f);
    let mut volume = remount(&mut f.disk);
    let epoch = volume.header().unwrap().epoch;
    let mut server = Server7::new(&mut volume);
    let reader = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    let receipt = writes[1].0;
    let workspace = receipt.workspace;
    let retry = |epoch: u64, key: u64| operation::Lookup::Retry {
        workspace,
        retry: Retry {
            epoch: Epoch::new(epoch).unwrap(),
            key: Key::new(key).unwrap(),
        },
    };
    let unknown_id = OperationId::new(LINEAGE, receipt.version.value() + 1).unwrap();
    let foreign_id = OperationId::new([0x11; 16], receipt.version.value()).unwrap();
    let foreign_workspace = Workspace::new([0x11; 16], f.workspace).unwrap();
    for (query, expected) in [
        (operation::Lookup::Id(unknown_id), Error::OutcomeUnknown),
        (retry(epoch, 0x999), Error::OutcomeUnknown),
        (retry(epoch + 1, 0x102), Error::ExpiredEpoch),
        (operation::Lookup::Id(foreign_id), Error::Lineage),
        (
            operation::Lookup::Retry {
                workspace: foreign_workspace,
                retry: receipt.retry,
            },
            Error::Lineage,
        ),
    ] {
        assert_eq!(
            reader
                .lookup(&mut server, &mut f.disk, query)
                .map(|(operation, _)| operation),
            Err(expected),
            "{query:?}"
        );
    }
    // Out of scope answers exactly like missing, in the current epoch and in
    // another one.
    let outside = Writer::grant(&mut server, 1, f.sibling, TRACKED_WRITE7, SUBJECT);
    for (query, expected) in [
        (retry(epoch, 0x102), Error::OutcomeUnknown),
        (retry(epoch, 0x999), Error::OutcomeUnknown),
        (retry(epoch + 1, 0x102), Error::ExpiredEpoch),
        (retry(epoch + 1, 0x999), Error::ExpiredEpoch),
        (operation::Lookup::Id(receipt.id), Error::OutcomeUnknown),
        (operation::Lookup::Id(unknown_id), Error::OutcomeUnknown),
    ] {
        assert_eq!(
            outside
                .lookup(&mut server, &mut f.disk, query)
                .map(|(operation, _)| operation),
            Err(expected),
            "out of scope {query:?}"
        );
    }
    // A profile-1 lookup (no marker) stays unsupported on V7.
    for legacy in [by_id(&receipt), by_retry(&receipt)] {
        let packet = legacy.packet(reader.context);
        assert!(matches!(packet.op, OPERATION_ID | OPERATION_RETRY));
        assert_eq!(
            status(reader.send(&mut server, &mut f.disk, packet)),
            Err(Error::Unsupported)
        );
    }
    // A part of a receipt this slot never looked up is not served.
    assert_eq!(
        reader.part(&mut server, &mut f.disk, receipt.id, 40),
        Err(Error::OutcomeUnknown)
    );
}

#[test]
fn lookups_are_bound_to_the_grant_subject_scope_and_inspect_right() {
    let mut f = fixture();
    let writes = two_writes(&mut f);
    let other_workspace = f
        .volume
        .create(&mut f.disk, 4, b"beta", Kind::Directory)
        .unwrap()
        .id;
    let mut volume = remount(&mut f.disk);
    let mut server = Server7::new(&mut volume);
    let receipt = writes[0].0;
    let reads = f.disk.reads;

    // Another subject has its own namespace: the same identities do not exist.
    let stranger = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT + 1);
    // Scopes that contain neither the workspace nor the object.
    let sibling_only = Writer::grant(&mut server, 1, f.sibling, TRACKED_WRITE7, SUBJECT);
    let elsewhere = Writer::grant(&mut server, 2, other_workspace, TRACKED_WRITE7, SUBJECT);
    for writer in [&stranger, &sibling_only, &elsewhere] {
        for query in [by_id(&receipt), by_retry(&receipt)] {
            assert_eq!(
                writer
                    .lookup(&mut server, &mut f.disk, query)
                    .map(|(operation, _)| operation),
                Err(Error::OutcomeUnknown)
            );
        }
    }
    // A read-only grant has neither the inspect right nor a subject.
    let reader = Writer::grant(&mut server, 3, f.workspace, READ_ONLY7, 0);
    for query in [by_id(&receipt), by_retry(&receipt)] {
        let reply = reader.send(
            &mut server,
            &mut f.disk,
            Lookup { query }.packet(reader.context),
        );
        assert_eq!(status(reply), Err(Error::Denied));
    }
    assert_eq!(f.disk.reads, reads, "refusals stream no snapshot");

    // The whole workspaces root and the object itself are both valid scopes.
    let root = Writer::grant(&mut server, 1, 4, TRACKED_WRITE7, SUBJECT);
    assert!(
        root.lookup(&mut server, &mut f.disk, by_id(&receipt))
            .is_ok()
    );
    let object = Writer::grant(&mut server, 2, f.file, TRACKED_WRITE7, SUBJECT);
    assert!(
        object
            .lookup(&mut server, &mut f.disk, by_retry(&receipt))
            .is_ok()
    );
}

#[test]
fn revoking_a_slot_forgets_the_looked_up_receipt() {
    let mut f = fixture();
    let writes = two_writes(&mut f);
    let mut volume = remount(&mut f.disk);
    let mut server = Server7::new(&mut volume);
    let receipt = writes[0].0;
    let reader = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    reader
        .lookup(&mut server, &mut f.disk, by_id(&receipt))
        .unwrap();
    server.revoke(0).unwrap();
    let again = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    assert_eq!(
        again.part(&mut server, &mut f.disk, receipt.id, 40),
        Err(Error::OutcomeUnknown)
    );
    // A fresh lookup on the new binding recomputes it.
    assert!(
        again
            .lookup(&mut server, &mut f.disk, by_id(&receipt))
            .is_ok()
    );
}
