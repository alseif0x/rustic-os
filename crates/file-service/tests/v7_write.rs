// SPDX-License-Identifier: Apache-2.0
//! Profile-2 tracked replacement through the V7 service: streamed chunks,
//! receipts, the retained-record budget, retry replay and authority loss, and
//! lookups of retained records by operation ID or retry identity, and the
//! owner's retention maintenance.
#[path = "v7_write/lookup.rs"]
mod lookup;
#[path = "v7_write/retention.rs"]
mod retention;

use rustic_abi::files::{
    DATA, Error, OPERATION_PART, Packet, REPLACE_ABORT, REPLACE_CHUNK, REPLACE_COMMIT,
    operation::{self, Key, OperationId, Retry},
    reference::{Epoch, References, Resource, Version, Workspace},
    workspace::{self, Lookup, Operation, RECEIPT_BYTES, Replacement},
};
use rustic_file_service::{GrantRequest7, READ_ONLY7, Server7, TRACKED_WRITE7};
use rustic_fs::{Disk, Error as FsError, Kind, Volume7};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const LINEAGE: [u8; 16] = [0x7b; 16];
const PEER: u64 = 11;
const SUBJECT: u64 = 2;

#[derive(Default)]
struct Sparse {
    sectors: BTreeMap<u64, [u8; 512]>,
    reads: usize,
    writes: usize,
    flushes: usize,
    /// When set, every write once `writes` reaches this count fails.
    fail_writes_from: Option<usize>,
}

impl Disk for Sparse {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), FsError> {
        self.reads += 1;
        *bytes = self.sectors.get(&sector).copied().unwrap_or([0; 512]);
        Ok(())
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), FsError> {
        if self
            .fail_writes_from
            .is_some_and(|limit| self.writes >= limit)
        {
            return Err(FsError::Io);
        }
        self.writes += 1;
        self.sectors.insert(sector, *bytes);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), FsError> {
        self.flushes += 1;
        Ok(())
    }
}

struct Fixture {
    volume: Volume7,
    disk: Sparse,
    workspace: u32,
    file: u32,
    sibling: u32,
}

fn fixture() -> Fixture {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    let workspace = volume
        .create(&mut disk, 4, b"alpha", Kind::Directory)
        .unwrap();
    let file = volume
        .create(&mut disk, workspace.id, b"scratch.bin", Kind::File)
        .unwrap();
    let sibling = volume
        .create(&mut disk, workspace.id, b"other", Kind::File)
        .unwrap();
    Fixture {
        volume,
        disk,
        workspace: workspace.id,
        file: file.id,
        sibling: sibling.id,
    }
}

fn authority(scope: u32, rights: u8, subject: u64) -> GrantRequest7 {
    GrantRequest7 {
        peer: PEER,
        endpoint: 90,
        scope,
        rights,
        subject,
        expires: 0,
    }
}

fn pattern(seed: u8, size: usize) -> Vec<u8> {
    (0..size)
        .map(|index| {
            seed.wrapping_mul(31)
                .wrapping_add((index * 7 + index / 509) as u8)
        })
        .collect()
}

/// One client binding: slot, context and the logical replacement identity.
struct Writer {
    slot: usize,
    context: u32,
}

impl Writer {
    fn grant(server: &mut Server7<'_>, slot: usize, scope: u32, rights: u8, subject: u64) -> Self {
        let grant = server
            .grant(slot, authority(scope, rights, subject))
            .unwrap();
        Self {
            slot,
            context: grant.context,
        }
    }

    fn send(&self, server: &mut Server7<'_>, disk: &mut Sparse, mut p: Packet) -> Packet {
        p.context = self.context;
        let reply = server.handle(disk, self.slot, PEER, p, 0);
        assert_eq!(reply.op, p.op);
        assert_eq!(reply.context, self.context);
        reply
    }

    fn open(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        replacement: operation::Replacement,
        size: usize,
    ) -> Result<(), Error> {
        let open = Replacement {
            request: replacement,
        }
        .packet(size, self.context)
        .unwrap();
        let reply = self.send(server, disk, open);
        status(reply)?;
        assert_eq!(
            (reply.id, reply.arg, reply.version, reply.count),
            (0, 0, 0, 0)
        );
        Ok(())
    }

    fn chunks(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        object: u32,
        bytes: &[u8],
    ) -> Result<(), Error> {
        for (index, chunk) in bytes.chunks(DATA).enumerate() {
            let mut p = Packet::new(REPLACE_CHUNK);
            p.id = object;
            p.arg = (index * DATA) as u32;
            p.count = chunk.len() as u8;
            p.data[..chunk.len()].copy_from_slice(chunk);
            status(self.send(server, disk, p))?;
        }
        Ok(())
    }

    fn commit(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        object: u32,
    ) -> Result<(Operation, [u8; RECEIPT_BYTES]), Error> {
        let mut p = Packet::new(REPLACE_COMMIT);
        p.id = object;
        let first = self.send(server, disk, p);
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

    fn part(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        id: OperationId,
        offset: usize,
    ) -> Result<Packet, Error> {
        let mut p = Lookup {
            query: operation::Lookup::Id(id),
        }
        .packet(self.context);
        p.op = OPERATION_PART;
        p.arg = offset as u32;
        let reply = self.send(server, disk, p);
        status(reply)?;
        Ok(reply)
    }

    fn replace(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        replacement: operation::Replacement,
        bytes: &[u8],
    ) -> Result<(Operation, [u8; RECEIPT_BYTES]), Error> {
        let object = replacement.resource.object();
        self.open(server, disk, replacement, bytes.len())?;
        self.chunks(server, disk, object, bytes)?;
        self.commit(server, disk, object)
    }

    fn abort(&self, server: &mut Server7<'_>, disk: &mut Sparse, object: u32) -> Result<(), Error> {
        let mut p = Packet::new(REPLACE_ABORT);
        p.id = object;
        status(self.send(server, disk, p))
    }
}

fn status(p: Packet) -> Result<(), Error> {
    if p.status == 0 {
        return Ok(());
    }
    assert_eq!((p.id, p.arg, p.version, p.count), (0, 0, 0, 0));
    Err(Error::parse(p.status).unwrap_err())
}

fn replacement(
    volume: &Volume7,
    workspace: u32,
    object: u32,
    version: u64,
    key: u64,
) -> operation::Replacement {
    let references = References::new(LINEAGE, workspace, object).unwrap();
    operation::Replacement {
        workspace: references.workspace,
        resource: references.resource,
        expected_version: Version::new(version).unwrap(),
        retry: Retry {
            epoch: Epoch::new(volume.header().unwrap().epoch).unwrap(),
            key: Key::new(key).unwrap(),
        },
    }
}

fn read_all(volume: &Volume7, disk: &mut Sparse, id: u32) -> Vec<u8> {
    let node = *volume.node(id).unwrap().unwrap();
    let mut bytes = vec![0; node.length as usize];
    let mut offset = 0;
    while offset < bytes.len() {
        let end = (offset + 1024).min(bytes.len());
        offset += volume
            .read_range(disk, id, None, offset as u64, &mut bytes[offset..end])
            .unwrap();
    }
    bytes
}

fn version(volume: &Volume7, id: u32) -> u64 {
    volume.node(id).unwrap().unwrap().version
}

#[test]
fn a_large_streamed_write_commits_the_exact_bytes_and_returns_its_receipt() {
    let mut f = fixture();
    let previous = version(&f.volume, f.file);
    let bytes = pattern(3, 200 * 1024 + 7);
    let request = replacement(&f.volume, f.workspace, f.file, previous, 41);
    let sequence_before = f.volume.header().unwrap().sequence;
    let mut server = Server7::new(&mut f.volume);
    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    let (receipt, encoded) = writer
        .replace(&mut server, &mut f.disk, request, &bytes)
        .unwrap();
    assert_eq!(server.volume().open_stages(), 0);
    let committed = sequence_before + 1;
    assert_eq!(receipt.workspace, request.workspace);
    assert_eq!(receipt.resource, request.resource);
    assert_eq!(receipt.retry, request.retry);
    assert_eq!(receipt.previous_version.value(), previous);
    assert_eq!(receipt.version.value(), committed);
    assert_eq!(receipt.id, OperationId::new(LINEAGE, committed).unwrap());
    assert_eq!(receipt.service_instance.sequence(), committed);
    assert_eq!(receipt.size as usize, bytes.len());
    assert_eq!(receipt.sha256, <[u8; 32]>::from(Sha256::digest(&bytes)));
    assert_eq!(encoded[68..72], workspace::PROFILE.to_le_bytes());
    drop(server);

    assert_eq!(version(&f.volume, f.file), committed);
    assert_eq!(read_all(&f.volume, &mut f.disk, f.file), bytes);
    let records: Vec<_> = f
        .volume
        .retained_records()
        .unwrap()
        .iter()
        .flatten()
        .copied()
        .collect();
    assert_eq!(records.len(), 1);
    let record = records[0];
    assert_eq!(
        (
            record.subject,
            record.workspace,
            record.object,
            record.retry_key
        ),
        (SUBJECT, f.workspace, f.file, 41)
    );
    assert_eq!((record.previous, record.committed), (previous, committed));
}

#[test]
fn stale_versions_are_refused_before_any_write_and_budget_exhaustion_is_full() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    let mut current = version(server.volume(), f.file);

    let writes_before = f.disk.writes;
    let stale = replacement(server.volume(), f.workspace, f.file, current + 50, 99);
    assert_eq!(
        writer.open(&mut server, &mut f.disk, stale, 513),
        Err(Error::Version)
    );
    assert_eq!(f.disk.writes, writes_before);
    assert_eq!(server.volume().open_stages(), 0);

    for key in 1..=8u64 {
        let bytes = pattern(key as u8, 513 * key as usize);
        let request = replacement(server.volume(), f.workspace, f.file, current, key);
        let (receipt, _) = writer
            .replace(&mut server, &mut f.disk, request, &bytes)
            .unwrap();
        current = receipt.version.value();
    }
    let request = replacement(server.volume(), f.workspace, f.file, current, 9);
    assert_eq!(
        writer.open(&mut server, &mut f.disk, request, 1),
        Err(Error::Full)
    );
    // The stale version is still reported ahead of the exhausted budget.
    let stale = replacement(server.volume(), f.workspace, f.file, current - 1, 9);
    assert_eq!(
        writer.open(&mut server, &mut f.disk, stale, 1),
        Err(Error::Version)
    );
    assert_eq!(server.volume().open_stages(), 0);
    drop(server);
    assert_eq!(
        read_all(&f.volume, &mut f.disk, f.file),
        pattern(8, 513 * 8)
    );
}

#[test]
fn revoking_or_detaching_mid_transfer_aborts_the_stage_and_frees_the_key() {
    let mut f = fixture();
    let original = version(&f.volume, f.file);
    let bytes = pattern(5, 4096 + 100);
    let request = replacement(&f.volume, f.workspace, f.file, original, 7);
    let mut server = Server7::new(&mut f.volume);
    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    writer
        .open(&mut server, &mut f.disk, request, bytes.len())
        .unwrap();
    writer
        .chunks(&mut server, &mut f.disk, f.file, &bytes[..2000])
        .unwrap();
    assert_eq!(server.volume().open_stages(), 1);
    server.revoke(0).unwrap();
    assert_eq!(server.volume().open_stages(), 0);
    assert_eq!(
        writer.commit(&mut server, &mut f.disk, f.file).unwrap_err(),
        Error::Revoked
    );
    assert_eq!(version(server.volume(), f.file), original);
    assert!(
        server
            .volume()
            .retained_records()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );

    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    writer
        .open(&mut server, &mut f.disk, request, bytes.len())
        .unwrap();
    writer
        .chunks(&mut server, &mut f.disk, f.file, &bytes[..40])
        .unwrap();
    server.detach(0);
    assert_eq!(server.volume().open_stages(), 0);

    let expiring = server
        .grant(
            0,
            GrantRequest7 {
                expires: 10,
                ..authority(f.workspace, TRACKED_WRITE7, SUBJECT)
            },
        )
        .unwrap();
    let writer = Writer {
        slot: 0,
        context: expiring.context,
    };
    writer
        .open(&mut server, &mut f.disk, request, bytes.len())
        .unwrap();
    assert_eq!(server.expire(10), 1);
    assert_eq!(server.volume().open_stages(), 0);

    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    let (receipt, _) = writer
        .replace(&mut server, &mut f.disk, request, &bytes)
        .unwrap();
    assert_eq!(receipt.previous_version.value(), original);
    drop(server);
    assert_eq!(read_all(&f.volume, &mut f.disk, f.file), bytes);
}

#[test]
fn an_exact_retry_after_remount_replays_the_receipt_without_writes() {
    let mut f = fixture();
    let bytes = pattern(9, 64 * 1024);
    let request = replacement(
        &f.volume,
        f.workspace,
        f.file,
        version(&f.volume, f.file),
        77,
    );
    let first = {
        let mut server = Server7::new(&mut f.volume);
        let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
        writer
            .replace(&mut server, &mut f.disk, request, &bytes)
            .unwrap()
    };

    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut f.disk).unwrap();
    let (writes, flushes) = (f.disk.writes, f.disk.flushes);
    let mut server = Server7::new(&mut volume);
    let writer = Writer::grant(&mut server, 2, f.workspace, TRACKED_WRITE7, SUBJECT);
    let replayed = writer
        .replace(&mut server, &mut f.disk, request, &bytes)
        .unwrap();
    assert_eq!(
        replayed.1, first.1,
        "the replayed receipt is byte-identical"
    );
    assert_eq!((f.disk.writes, f.disk.flushes), (writes, flushes));

    let mut different = bytes.clone();
    *different.last_mut().unwrap() ^= 1;
    assert_eq!(
        writer
            .replace(&mut server, &mut f.disk, request, &different)
            .unwrap_err(),
        Error::IdempotencyConflict
    );
    assert_eq!(
        writer.open(&mut server, &mut f.disk, request, bytes.len() - 1),
        Err(Error::IdempotencyConflict)
    );
    assert_eq!((f.disk.writes, f.disk.flushes), (writes, flushes));
    assert_eq!(server.volume().open_stages(), 0);

    // Another subject has a separate retry scope: the same key is a fresh write.
    let other = Writer::grant(&mut server, 1, f.workspace, TRACKED_WRITE7, SUBJECT + 1);
    let fresh = replacement(
        server.volume(),
        f.workspace,
        f.file,
        first.0.version.value(),
        77,
    );
    let (receipt, _) = other
        .replace(&mut server, &mut f.disk, fresh, b"other subject")
        .unwrap();
    assert!(receipt.version.value() > first.0.version.value());
    assert_ne!(receipt.service_instance, first.0.service_instance);
}

#[test]
fn write_authority_requires_the_tracked_profile_a_subject_and_scope() {
    let mut f = fixture();
    let current = version(&f.volume, f.file);
    let request = replacement(&f.volume, f.workspace, f.file, current, 3);
    let mut server = Server7::new(&mut f.volume);
    for (rights, subject) in [
        (READ_ONLY7, 1),
        (TRACKED_WRITE7, 0),
        (0b011, 1),
        (0b101, 1),
        (0b1111, 1),
    ] {
        assert_eq!(
            server.grant(0, authority(f.workspace, rights, subject)),
            Err(Error::Invalid),
            "rights {rights:#b} subject {subject}"
        );
    }

    let reader = Writer::grant(&mut server, 0, f.workspace, READ_ONLY7, 0);
    let writes = f.disk.writes;
    assert_eq!(
        reader.open(&mut server, &mut f.disk, request, 10),
        Err(Error::Denied)
    );
    assert_eq!(
        reader.chunks(&mut server, &mut f.disk, f.file, b"x"),
        Err(Error::Denied)
    );
    assert_eq!(
        reader.abort(&mut server, &mut f.disk, f.file),
        Err(Error::Denied)
    );

    let scoped = Writer::grant(&mut server, 1, f.sibling, TRACKED_WRITE7, SUBJECT);
    assert_eq!(
        scoped.open(&mut server, &mut f.disk, request, 10),
        Err(Error::Denied)
    );
    let foreign = operation::Replacement {
        workspace: Workspace::new([0x11; 16], f.workspace).unwrap(),
        resource: Resource::new(Workspace::new([0x11; 16], f.workspace).unwrap(), f.file).unwrap(),
        ..request
    };
    let writer = Writer::grant(&mut server, 2, f.workspace, TRACKED_WRITE7, SUBJECT);
    assert_eq!(
        writer.open(&mut server, &mut f.disk, foreign, 10),
        Err(Error::Denied)
    );
    assert_eq!(f.disk.writes, writes);
    assert_eq!(server.volume().open_stages(), 0);
}

#[test]
fn transfers_bind_offsets_objects_slots_and_the_two_stage_limit() {
    let mut f = fixture();
    let current = version(&f.volume, f.file);
    let sibling_version = version(&f.volume, f.sibling);
    let request = replacement(&f.volume, f.workspace, f.file, current, 1);
    let mut server = Server7::new(&mut f.volume);
    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    writer.open(&mut server, &mut f.disk, request, 100).unwrap();
    assert_eq!(
        writer.open(&mut server, &mut f.disk, request, 100),
        Err(Error::Busy)
    );
    assert_eq!(
        writer.chunks(&mut server, &mut f.disk, f.sibling, b"x"),
        Err(Error::NoTransfer)
    );
    let mut skipped = Packet::new(REPLACE_CHUNK);
    skipped.id = f.file;
    skipped.arg = 40;
    skipped.count = 1;
    assert_eq!(
        status(writer.send(&mut server, &mut f.disk, skipped)),
        Err(Error::Offset)
    );
    // The refused offset left the transfer usable at offset zero; an
    // incomplete transfer cannot commit.
    writer
        .chunks(&mut server, &mut f.disk, f.file, &[1; 60])
        .unwrap();
    assert_eq!(
        writer.commit(&mut server, &mut f.disk, f.file).unwrap_err(),
        Error::Offset
    );

    let other = Writer::grant(&mut server, 1, f.workspace, TRACKED_WRITE7, SUBJECT);
    let second = replacement(server.volume(), f.workspace, f.sibling, sibling_version, 2);
    other.open(&mut server, &mut f.disk, second, 10).unwrap();
    let third = Writer::grant(&mut server, 2, f.workspace, TRACKED_WRITE7, SUBJECT);
    let busy = replacement(server.volume(), f.workspace, f.sibling, sibling_version, 3);
    assert_eq!(
        third.open(&mut server, &mut f.disk, busy, 10),
        Err(Error::Busy)
    );
    other.abort(&mut server, &mut f.disk, f.sibling).unwrap();
    writer.abort(&mut server, &mut f.disk, f.file).unwrap();
    assert_eq!(server.volume().open_stages(), 0);

    let (receipt, _) = third
        .replace(&mut server, &mut f.disk, busy, b"0123456789")
        .unwrap();
    assert_eq!(
        other.part(&mut server, &mut f.disk, receipt.id, 40),
        Err(Error::OutcomeUnknown),
        "a receipt is visible only to the slot that produced it"
    );
    assert!(third.part(&mut server, &mut f.disk, receipt.id, 80).is_ok());
    let reader = Writer::grant(&mut server, 3, f.workspace, READ_ONLY7, 0);
    assert_eq!(
        reader.part(&mut server, &mut f.disk, receipt.id, 40),
        Err(Error::Denied)
    );
}

#[test]
fn an_empty_replacement_commits_without_chunks() {
    let mut f = fixture();
    let request = replacement(
        &f.volume,
        f.workspace,
        f.file,
        version(&f.volume, f.file),
        5,
    );
    let mut server = Server7::new(&mut f.volume);
    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    let (receipt, _) = writer
        .replace(&mut server, &mut f.disk, request, b"x")
        .unwrap();
    let emptied = replacement(
        server.volume(),
        f.workspace,
        f.file,
        receipt.version.value(),
        6,
    );
    let (empty, _) = writer
        .replace(&mut server, &mut f.disk, emptied, b"")
        .unwrap();
    assert_eq!(empty.size, 0);
    assert_eq!(empty.sha256, <[u8; 32]>::from(Sha256::digest(b"")));
    drop(server);
    assert!(read_all(&f.volume, &mut f.disk, f.file).is_empty());
}

#[test]
fn a_failed_sector_write_drops_the_transfer_and_fences_the_volume() {
    let mut f = fixture();
    let original = version(&f.volume, f.file);
    let bytes = pattern(4, 2048);
    let request = replacement(&f.volume, f.workspace, f.file, original, 11);
    let mut server = Server7::new(&mut f.volume);
    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    writer
        .open(&mut server, &mut f.disk, request, bytes.len())
        .unwrap();
    // The first sector is staged; the second sector's write fails mid-chunk
    // (bytes 1000..1040 complete the sector at 1024).
    writer
        .chunks(&mut server, &mut f.disk, f.file, &bytes[..1000])
        .unwrap();
    f.disk.fail_writes_from = Some(f.disk.writes);
    let mut crossing = Packet::new(REPLACE_CHUNK);
    crossing.id = f.file;
    crossing.arg = 1000;
    crossing.count = 40;
    crossing.data.copy_from_slice(&bytes[1000..1040]);
    assert_eq!(
        status(writer.send(&mut server, &mut f.disk, crossing)),
        Err(Error::Uncertain)
    );
    assert_eq!(server.volume().open_stages(), 0);
    f.disk.fail_writes_from = None;
    let writes = f.disk.writes;

    // The transfer is gone, whatever offset the client continues with.
    for offset in [1000u32, 1040] {
        let mut next = Packet::new(REPLACE_CHUNK);
        next.id = f.file;
        next.arg = offset;
        next.count = 1;
        assert_eq!(
            status(writer.send(&mut server, &mut f.disk, next)),
            Err(Error::NoTransfer)
        );
    }
    assert_eq!(
        writer.commit(&mut server, &mut f.disk, f.file).unwrap_err(),
        Error::NoTransfer
    );
    // The fenced volume refuses new transfers and reads until remount.
    assert_eq!(
        writer.open(&mut server, &mut f.disk, request, bytes.len()),
        Err(Error::Uncertain)
    );
    let read = rustic_abi::files::read::Request {
        workspace: request.workspace,
        resource: request.resource,
        expected_version: None,
        offset: 0,
        length: 1,
    }
    .packet(rustic_abi::files::READ_OPEN, writer.context)
    .unwrap();
    assert_eq!(
        status(writer.send(&mut server, &mut f.disk, read)),
        Err(Error::Uncertain)
    );
    assert_eq!(f.disk.writes, writes);
    drop(server);

    // Nothing was published: a remount sees the original file and no record.
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut f.disk).unwrap();
    assert_eq!(version(&volume, f.file), original);
    assert!(
        volume
            .retained_records()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn a_resent_chunk_is_refused_and_the_transfer_continues_at_the_right_offset() {
    let mut f = fixture();
    let bytes = pattern(6, 600);
    let request = replacement(
        &f.volume,
        f.workspace,
        f.file,
        version(&f.volume, f.file),
        12,
    );
    let mut server = Server7::new(&mut f.volume);
    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    writer
        .open(&mut server, &mut f.disk, request, bytes.len())
        .unwrap();
    writer
        .chunks(&mut server, &mut f.disk, f.file, &bytes[..80])
        .unwrap();
    let mut duplicate = Packet::new(REPLACE_CHUNK);
    duplicate.id = f.file;
    duplicate.arg = 40;
    duplicate.count = 40;
    duplicate.data.copy_from_slice(&bytes[40..80]);
    let writes = f.disk.writes;
    assert_eq!(
        status(writer.send(&mut server, &mut f.disk, duplicate)),
        Err(Error::Offset)
    );
    assert_eq!(f.disk.writes, writes);
    for (index, chunk) in bytes[80..].chunks(DATA).enumerate() {
        let mut p = Packet::new(REPLACE_CHUNK);
        p.id = f.file;
        p.arg = (80 + index * DATA) as u32;
        p.count = chunk.len() as u8;
        p.data[..chunk.len()].copy_from_slice(chunk);
        status(writer.send(&mut server, &mut f.disk, p)).unwrap();
    }
    let (receipt, _) = writer.commit(&mut server, &mut f.disk, f.file).unwrap();
    assert_eq!(
        receipt.sha256,
        <[u8; 32]>::from(Sha256::digest(&bytes)),
        "the refused duplicate was not counted twice"
    );
    drop(server);
    assert_eq!(read_all(&f.volume, &mut f.disk, f.file), bytes);
}
