// SPDX-License-Identifier: Apache-2.0
//! Existing flat tracked writes over V7's scoped retained-record table.

use rustic_abi::files::{
    ABORT, CHUNK, COMMIT, DATA, Error, Packet, RECEIPT, REPLACE_ABORT, REPLACE_CHUNK,
    REPLACE_COMMIT, TRACK_BEGIN, operation,
    recovery::{Receipt, Retry},
    reference::{Epoch, Resource, Version, Workspace},
};
use rustic_file_service::{GrantRequest7, Server7, TRACKED_WRITE7};
use rustic_fs::{Disk, Error as FsError, Kind, Volume7};
use std::collections::BTreeMap;

const LINEAGE: [u8; 16] = [0x4a; 16];
const SUBJECT: u64 = 17;
const PEER_BASE: u64 = 31;

#[derive(Default)]
struct Sparse {
    sectors: BTreeMap<u64, [u8; 512]>,
    reads: usize,
    writes: usize,
    flushes: usize,
}

impl Disk for Sparse {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), FsError> {
        self.reads += 1;
        *bytes = self.sectors.get(&sector).copied().unwrap_or([0; 512]);
        Ok(())
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), FsError> {
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
    other_workspace: u32,
    file: u32,
    other_file: u32,
}

fn fixture() -> Fixture {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    let workspace = volume
        .create(&mut disk, 4, b"alpha", Kind::Directory)
        .unwrap();
    let other_workspace = volume
        .create(&mut disk, 4, b"beta", Kind::Directory)
        .unwrap();
    let file = volume
        .create(&mut disk, workspace.id, b"first", Kind::File)
        .unwrap();
    let other_file = volume
        .create(&mut disk, other_workspace.id, b"second", Kind::File)
        .unwrap();
    Fixture {
        volume,
        disk,
        workspace: workspace.id,
        other_workspace: other_workspace.id,
        file: file.id,
        other_file: other_file.id,
    }
}

fn grant(server: &mut Server7<'_>, slot: usize, scope: u32, rights: u8) -> u32 {
    server
        .grant(
            slot,
            GrantRequest7 {
                peer: PEER_BASE + slot as u64,
                endpoint: 70 + slot as u64,
                scope,
                rights,
                subject: SUBJECT,
                expires: 0,
            },
        )
        .unwrap()
        .context
}

fn send(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    slot: usize,
    context: u32,
    mut packet: Packet,
) -> Packet {
    packet.context = context;
    let reply = server.handle(disk, slot, PEER_BASE + slot as u64, packet, 0);
    assert_eq!(reply.op, packet.op);
    assert_eq!(reply.context, context);
    reply
}

fn ok(packet: Packet) -> Packet {
    if packet.status == 0 {
        packet
    } else {
        panic!(
            "unexpected file-service error: {:?}",
            Error::parse(packet.status)
        );
    }
}

fn retry(key: u64) -> Retry {
    Retry {
        lineage: LINEAGE,
        epoch: 1,
        key,
    }
}

fn begin_packet(id: u32, version: u64, retry: Retry, bytes: &[u8]) -> Packet {
    let mut packet = Packet::new(TRACK_BEGIN);
    packet.id = id;
    packet.version = version;
    packet.arg = bytes.len() as u32;
    packet.count = 32;
    packet.data[..32].copy_from_slice(&retry.encode());
    packet
}

fn legacy_chunks(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    slot: usize,
    context: u32,
    id: u32,
    bytes: &[u8],
) {
    for (index, chunk) in bytes.chunks(DATA).enumerate() {
        let mut packet = Packet::new(CHUNK);
        packet.id = id;
        packet.arg = (index * DATA) as u32;
        packet.count = chunk.len() as u8;
        packet.data[..chunk.len()].copy_from_slice(chunk);
        ok(send(server, disk, slot, context, packet));
    }
}

fn legacy_replace(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    slot: usize,
    context: u32,
    target: (u32, u64),
    retry: Retry,
    bytes: &[u8],
) -> Result<Receipt, Error> {
    let (id, version) = target;
    let opened = send(
        server,
        disk,
        slot,
        context,
        begin_packet(id, version, retry, bytes),
    );
    if opened.status != 0 {
        return Err(Error::parse(opened.status).unwrap_err());
    }
    legacy_chunks(server, disk, slot, context, id, bytes);
    let mut commit = Packet::new(COMMIT);
    commit.id = id;
    let result = send(server, disk, slot, context, commit);
    if result.status != 0 {
        return Err(Error::parse(result.status).unwrap_err());
    }
    assert_eq!(result.count, 40);
    Receipt::decode(result)
}

fn profile1_request(
    lineage: [u8; 16],
    workspace: u32,
    object: u32,
    version: u64,
    key: u64,
) -> operation::Replacement {
    let workspace = Workspace::new(lineage, workspace).unwrap();
    operation::Replacement {
        workspace,
        resource: Resource::new(workspace, object).unwrap(),
        expected_version: Version::new(version).unwrap(),
        retry: operation::Retry {
            epoch: Epoch::new(1).unwrap(),
            key: operation::Key::new(key).unwrap(),
        },
    }
}

fn profile1_replace(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    slot: usize,
    context: u32,
    request: operation::Replacement,
    bytes: &[u8],
) {
    ok(send(
        server,
        disk,
        slot,
        context,
        request.packet(bytes.len(), context).unwrap(),
    ));
    for (index, chunk) in bytes.chunks(DATA).enumerate() {
        let mut packet = Packet::new(REPLACE_CHUNK);
        packet.id = request.resource.object();
        packet.arg = (index * DATA) as u32;
        packet.count = chunk.len() as u8;
        packet.data[..chunk.len()].copy_from_slice(chunk);
        ok(send(server, disk, slot, context, packet));
    }
    let mut commit = Packet::new(REPLACE_COMMIT);
    commit.id = request.resource.object();
    ok(send(server, disk, slot, context, commit));
}

#[test]
fn flat_write_records_the_canonical_root_and_replays_with_inspection_only() {
    let mut f = fixture();
    let previous = f.volume.stat(f.file).unwrap().version;
    let mut server = Server7::new(&mut f.volume);
    let writer = grant(&mut server, 0, f.workspace, TRACKED_WRITE7);
    let key = retry(501);
    let first = legacy_replace(
        &mut server,
        &mut f.disk,
        0,
        writer,
        (f.file, previous),
        key,
        b"legacy",
    )
    .unwrap();
    assert_eq!(first.id, f.file);
    assert_eq!(first.retry, key);
    assert_eq!(first.previous, previous);
    assert_eq!(first.length, 6);
    let stored = server.volume().retained_records().unwrap()[0].unwrap();
    assert_eq!((stored.workspace, stored.object), (4, f.file));
    drop(server);

    let mut server = Server7::new(&mut f.volume);
    let inspector = grant(
        &mut server,
        0,
        f.workspace,
        rustic_abi::files::INSPECT_RIGHT,
    );
    let writes = f.disk.writes;
    let flushes = f.disk.flushes;

    let mut info = Packet::new(rustic_abi::files::RECOVERY);
    info.id = f.workspace;
    let info = ok(send(&mut server, &mut f.disk, 0, inspector, info));
    assert_eq!(info.count, 24);
    assert_eq!(info.arg, 8);
    assert_eq!(&info.data[..16], &LINEAGE);
    assert_eq!(u64::from_le_bytes(info.data[16..24].try_into().unwrap()), 1);

    let mut lookup = Packet::new(RECEIPT);
    lookup.id = f.file;
    lookup.count = 32;
    lookup.data[..32].copy_from_slice(&key.encode());
    let replayed =
        Receipt::decode(ok(send(&mut server, &mut f.disk, 0, inspector, lookup))).unwrap();
    assert_eq!(replayed, first);

    assert_eq!(
        legacy_replace(
            &mut server,
            &mut f.disk,
            0,
            inspector,
            (f.file, previous),
            key,
            b"legacy",
        ),
        Ok(first),
        "an exact retry is readable and byte-verified with INSPECT alone"
    );
    assert_eq!((f.disk.writes, f.disk.flushes), (writes, flushes));

    let denied = send(
        &mut server,
        &mut f.disk,
        0,
        inspector,
        begin_packet(f.file, first.committed, retry(502), b"fresh"),
    );
    assert_eq!(denied.status, Error::Denied as u8);
    assert_eq!(server.volume().open_stages(), 0);
}

#[test]
fn flat_collision_after_begin_refuses_stale_commit_and_cross_framing_preserves_stages() {
    let mut f = fixture();
    let previous = f.volume.stat(f.file).unwrap().version;
    let mut server = Server7::new(&mut f.volume);
    let legacy_context = grant(&mut server, 0, f.workspace, TRACKED_WRITE7);
    let modern_context = grant(&mut server, 1, f.workspace, TRACKED_WRITE7);
    let key = retry(700);

    ok(send(
        &mut server,
        &mut f.disk,
        0,
        legacy_context,
        begin_packet(f.file, previous, key, b"legacy"),
    ));
    let mut wrong_profile = Packet::new(REPLACE_CHUNK);
    wrong_profile.id = f.file;
    wrong_profile.count = 1;
    wrong_profile.data[0] = b'x';
    assert_eq!(
        send(&mut server, &mut f.disk, 0, legacy_context, wrong_profile,).status,
        Error::NoTransfer as u8
    );
    assert_eq!(
        send(
            &mut server,
            &mut f.disk,
            0,
            legacy_context,
            Packet {
                id: f.file,
                ..Packet::new(REPLACE_ABORT)
            },
        )
        .status,
        Error::NoTransfer as u8
    );
    legacy_chunks(
        &mut server,
        &mut f.disk,
        0,
        legacy_context,
        f.file,
        b"legacy",
    );

    let scoped = profile1_request(LINEAGE, f.workspace, f.file, previous, key.key);
    ok(send(
        &mut server,
        &mut f.disk,
        1,
        modern_context,
        scoped.packet(b"modern".len(), modern_context).unwrap(),
    ));

    let mut wrong_legacy_chunk = Packet::new(CHUNK);
    wrong_legacy_chunk.id = f.file;
    wrong_legacy_chunk.count = 1;
    wrong_legacy_chunk.data[0] = b'x';
    assert_eq!(
        send(
            &mut server,
            &mut f.disk,
            1,
            modern_context,
            wrong_legacy_chunk,
        )
        .status,
        Error::NoTransfer as u8
    );
    assert_eq!(
        send(
            &mut server,
            &mut f.disk,
            1,
            modern_context,
            Packet {
                id: f.file,
                ..Packet::new(ABORT)
            },
        )
        .status,
        Error::NoTransfer as u8
    );

    for (index, chunk) in b"modern".chunks(DATA).enumerate() {
        let mut packet = Packet::new(REPLACE_CHUNK);
        packet.id = f.file;
        packet.arg = (index * DATA) as u32;
        packet.count = chunk.len() as u8;
        packet.data[..chunk.len()].copy_from_slice(chunk);
        ok(send(&mut server, &mut f.disk, 1, modern_context, packet));
    }
    let mut modern_commit = Packet::new(REPLACE_COMMIT);
    modern_commit.id = f.file;
    ok(send(
        &mut server,
        &mut f.disk,
        1,
        modern_context,
        modern_commit,
    ));

    let writes = f.disk.writes;
    let flushes = f.disk.flushes;
    let mut legacy_commit = Packet::new(COMMIT);
    legacy_commit.id = f.file;
    assert_eq!(
        send(&mut server, &mut f.disk, 0, legacy_context, legacy_commit,).status,
        Error::IdempotencyConflict as u8
    );
    assert_eq!((f.disk.writes, f.disk.flushes), (writes, flushes));
    assert_eq!(server.volume().open_stages(), 0);
    let records = server
        .volume()
        .retained_records()
        .unwrap()
        .iter()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].workspace, f.workspace);

    let current_version = server.volume().stat(f.file).unwrap().version;
    ok(send(
        &mut server,
        &mut f.disk,
        0,
        legacy_context,
        begin_packet(f.file, current_version, retry(701), b"discard"),
    ));
    let mut legacy_abort = Packet::new(ABORT);
    legacy_abort.id = f.file;
    ok(send(
        &mut server,
        &mut f.disk,
        0,
        legacy_context,
        legacy_abort,
    ));
    assert_eq!(server.volume().open_stages(), 0);
}

#[test]
fn ambiguous_flat_key_across_scoped_records_is_refused_without_a_receipt() {
    let mut f = fixture();
    let first_version = f.volume.stat(f.file).unwrap().version;
    let other_version = f.volume.stat(f.other_file).unwrap().version;
    let mut server = Server7::new(&mut f.volume);
    let context = grant(&mut server, 0, 4, TRACKED_WRITE7);
    let key = 900;

    profile1_replace(
        &mut server,
        &mut f.disk,
        0,
        context,
        profile1_request(LINEAGE, f.workspace, f.file, first_version, key),
        b"first",
    );
    profile1_replace(
        &mut server,
        &mut f.disk,
        0,
        context,
        profile1_request(LINEAGE, f.other_workspace, f.other_file, other_version, key),
        b"other",
    );

    let mut lookup = Packet::new(RECEIPT);
    lookup.id = f.file;
    lookup.count = 32;
    lookup.data[..32].copy_from_slice(&retry(key).encode());
    assert_eq!(
        send(&mut server, &mut f.disk, 0, context, lookup).status,
        Error::IdempotencyConflict as u8
    );

    let current = server.volume().stat(f.file).unwrap().version;
    let ambiguous = send(
        &mut server,
        &mut f.disk,
        0,
        context,
        begin_packet(f.file, current, retry(key), b"first"),
    );
    assert_eq!(ambiguous.status, Error::IdempotencyConflict as u8);
    assert_eq!(ambiguous.data, [0; DATA]);
    assert_eq!(server.volume().open_stages(), 0);
}

#[test]
fn hidden_collisions_and_removed_companions_do_not_disclose_flat_receipts() {
    use rustic_abi::files::{INSPECT_RIGHT, REMOVE};
    let mut f = fixture();
    let first_version = f.volume.stat(f.file).unwrap().version;
    let other_version = f.volume.stat(f.other_file).unwrap().version;
    let mut server = Server7::new(&mut f.volume);
    let writer = grant(&mut server, 0, 4, TRACKED_WRITE7);
    for (workspace, object, version) in [
        (f.workspace, f.file, first_version),
        (f.other_workspace, f.other_file, other_version),
    ] {
        profile1_replace(
            &mut server,
            &mut f.disk,
            0,
            writer,
            profile1_request(LINEAGE, workspace, object, version, 901),
            b"scoped",
        );
    }
    let observer = grant(&mut server, 1, f.workspace, INSPECT_RIGHT);
    let mut query = Packet::new(RECEIPT);
    query.id = f.file;
    query.count = 32;
    query.data[..32].copy_from_slice(&retry(901).encode());
    let io = (f.disk.reads, f.disk.writes, f.disk.flushes);
    let hidden = send(&mut server, &mut f.disk, 1, observer, query);
    assert_eq!(hidden.status, Error::OutcomeUnknown as u8);
    assert_eq!((hidden.id, hidden.count, hidden.data), (0, 0, [0; DATA]));
    assert_eq!((f.disk.reads, f.disk.writes, f.disk.flushes), io);

    let previous = server.volume().stat(f.file).unwrap().version;
    let retained = legacy_replace(
        &mut server,
        &mut f.disk,
        0,
        writer,
        (f.file, previous),
        retry(902),
        b"retained",
    )
    .unwrap();
    let exact = grant(&mut server, 2, f.file, INSPECT_RIGHT);
    let companion = grant(&mut server, 3, f.other_workspace, INSPECT_RIGHT);
    let companion = server.extend(3, companion, f.file).unwrap();
    let mut remove = Packet::new(REMOVE);
    remove.id = f.file;
    ok(send(&mut server, &mut f.disk, 0, writer, remove));
    query.data[..32].copy_from_slice(&retry(902).encode());
    let io = (f.disk.reads, f.disk.writes, f.disk.flushes);
    let owned = send(&mut server, &mut f.disk, 2, exact, query);
    assert_eq!(Receipt::decode(ok(owned)), Ok(retained));
    let removed = send(&mut server, &mut f.disk, 3, companion, query);
    assert_eq!(removed.status, Error::OutcomeUnknown as u8);
    assert_eq!((removed.id, removed.count, removed.data), (0, 0, [0; DATA]));
    assert_eq!((f.disk.reads, f.disk.writes, f.disk.flushes), io);
}
