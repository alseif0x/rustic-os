// SPDX-License-Identifier: Apache-2.0
//! Existing namespace mutations and bounded untracked writes over V7.
use rustic_abi::files::{
    BEGIN, CHUNK, COMMIT, CREATE, DATA, Error, MAX_INLINE, MKDIR, Packet, READ, REMOVE, admission,
    operation::{self, Key, Retry},
    reference::{Epoch, References, Version},
    workspace,
};
use rustic_file_service::{Grant7, GrantRequest7, READ_ONLY7, Server7, TRACKED_WRITE7};
use rustic_fs::{Disk, Error as FsError, Kind, Volume7, WriteIdentity7};
use std::collections::BTreeMap;

const LINEAGE: [u8; 16] = [0x6d; 16];
const PEER: u64 = 21;
const SUBJECT: u64 = 8;
const ROOT: u32 = 4;

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
    directory: u32,
    file: u32,
    sibling: u32,
}

fn fixture(initial: &[u8]) -> Fixture {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    let workspace = volume
        .create(&mut disk, ROOT, b"alpha", Kind::Directory)
        .unwrap();
    let directory = volume
        .create(&mut disk, workspace.id, b"nested", Kind::Directory)
        .unwrap();
    let file = volume
        .create(&mut disk, directory.id, b"note", Kind::File)
        .unwrap();
    let sibling = volume
        .create(&mut disk, workspace.id, b"peer", Kind::File)
        .unwrap();
    if !initial.is_empty() {
        volume
            .replace(&mut disk, file.id, file.version, initial)
            .unwrap();
    }
    Fixture {
        volume,
        disk,
        workspace: workspace.id,
        directory: directory.id,
        file: file.id,
        sibling: sibling.id,
    }
}

fn authority(scope: u32, rights: u8, subject: u64, expires: u64, slot: usize) -> GrantRequest7 {
    GrantRequest7 {
        peer: PEER,
        endpoint: 90 + slot as u64,
        scope,
        rights,
        subject,
        expires,
    }
}

fn grant(server: &mut Server7<'_>, slot: usize, scope: u32, rights: u8, expires: u64) -> Grant7 {
    let subject = if rights == READ_ONLY7 { 0 } else { SUBJECT };
    server
        .grant(slot, authority(scope, rights, subject, expires, slot))
        .unwrap()
}

fn send(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    slot: usize,
    context: u32,
    mut packet: Packet,
    now: u64,
) -> Packet {
    packet.context = context;
    let reply = server.handle(disk, slot, PEER, packet, now);
    assert_eq!(reply.op, packet.op);
    assert_eq!(reply.context, context);
    reply
}

fn result(packet: Packet) -> Result<Packet, Error> {
    if packet.status == 0 {
        return Ok(packet);
    }
    assert_eq!(
        (packet.id, packet.arg, packet.version, packet.count),
        (0, 0, 0, 0)
    );
    assert_eq!(packet.data, [0; DATA]);
    Err(Error::parse(packet.status).unwrap_err())
}

fn begin(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    slot: usize,
    context: u32,
    id: u32,
    length: usize,
    version: u64,
) -> Result<(), Error> {
    let mut packet = Packet::new(BEGIN);
    packet.id = id;
    packet.arg = length as u32;
    packet.version = version;
    result(send(server, disk, slot, context, packet, 0)).map(|_| ())
}

// Keep each client binding and wire field explicit in the failure matrix.
#[allow(clippy::too_many_arguments)]
fn chunk(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    slot: usize,
    context: u32,
    op: u8,
    id: u32,
    offset: usize,
    bytes: &[u8],
) -> Result<(), Error> {
    assert!(!bytes.is_empty() && bytes.len() <= DATA);
    let mut packet = Packet::new(op);
    packet.id = id;
    packet.arg = offset as u32;
    packet.count = bytes.len() as u8;
    packet.data[..bytes.len()].copy_from_slice(bytes);
    result(send(server, disk, slot, context, packet, 0)).map(|_| ())
}

fn chunks(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    slot: usize,
    context: u32,
    op: u8,
    id: u32,
    bytes: &[u8],
) -> Result<(), Error> {
    for (index, part) in bytes.chunks(DATA).enumerate() {
        chunk(server, disk, slot, context, op, id, index * DATA, part)?;
    }
    Ok(())
}

fn bare(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    slot: usize,
    context: u32,
    op: u8,
    id: u32,
) -> Result<Packet, Error> {
    let mut packet = Packet::new(op);
    packet.id = id;
    result(send(server, disk, slot, context, packet, 0))
}

fn version(server: &Server7<'_>, id: u32) -> u64 {
    server.volume().stat(id).unwrap().version
}

fn read_all(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    slot: usize,
    context: u32,
    id: u32,
    length: usize,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(length);
    while bytes.len() < length {
        let offset = bytes.len();
        let mut packet = Packet::new(READ);
        packet.id = id;
        packet.arg = offset as u32;
        let reply = result(send(server, disk, slot, context, packet, 0)).unwrap();
        assert_eq!(reply.id, id);
        assert_eq!(reply.arg as usize, length);
        let take = reply.count as usize;
        assert!(take > 0 && take <= DATA);
        bytes.extend_from_slice(&reply.data[..take]);
    }
    bytes
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

fn profile_open(request: operation::Replacement, context: u32, op: u8) -> Packet {
    let mut packet = workspace::Replacement { request }
        .packet(1, context)
        .unwrap();
    packet.op = op;
    packet
}

#[test]
fn namespace_mutations_require_write_authority_and_verified_parent_scope() {
    let mut f = fixture(b"seed");
    let mut server = Server7::new(&mut f.volume);
    let writer = grant(&mut server, 0, f.workspace, TRACKED_WRITE7, 0);
    let reader = grant(&mut server, 1, f.workspace, READ_ONLY7, 0);

    let mut create = Packet::new(CREATE);
    create.id = f.directory;
    create.count = 4;
    create.data[..4].copy_from_slice(b"new\0");
    // The existing name grammar rejects embedded NUL, and the failed request
    // leaves the namespace unchanged.
    assert_eq!(
        result(send(&mut server, &mut f.disk, 0, writer.context, create, 0)).unwrap_err(),
        Error::Invalid
    );

    let mut create = Packet::new(CREATE);
    create.id = f.directory;
    create.count = 3;
    create.data[..3].copy_from_slice(b"new");
    let created = result(send(&mut server, &mut f.disk, 0, writer.context, create, 0)).unwrap();
    assert_eq!((created.count, created.arg, created.data[2]), (40, 0, 3));
    assert_eq!(&created.data[8..11], b"new");
    let new_file = created.id;

    let mut mkdir = Packet::new(MKDIR);
    mkdir.id = f.directory;
    mkdir.count = 3;
    mkdir.data[..3].copy_from_slice(b"dir");
    let new_dir = result(send(&mut server, &mut f.disk, 0, writer.context, mkdir, 0)).unwrap();
    assert_eq!(new_dir.data[0], Kind::Directory as u8);

    let before_root_mutation = (f.disk.reads, f.disk.writes, f.disk.flushes);
    let mut root_create = Packet::new(CREATE);
    root_create.count = 3;
    root_create.data[..3].copy_from_slice(b"bad");
    assert_eq!(
        result(send(
            &mut server,
            &mut f.disk,
            0,
            writer.context,
            root_create,
            0,
        ))
        .unwrap_err(),
        Error::Denied,
        "the virtual root is read-only"
    );
    assert_eq!(
        (f.disk.reads, f.disk.writes, f.disk.flushes),
        before_root_mutation
    );

    let file_scope = grant(&mut server, 2, f.file, TRACKED_WRITE7, 0);
    let mut ancestor_create = Packet::new(CREATE);
    ancestor_create.id = f.directory;
    ancestor_create.count = 3;
    ancestor_create.data[..3].copy_from_slice(b"bad");
    assert_eq!(
        result(send(
            &mut server,
            &mut f.disk,
            2,
            file_scope.context,
            ancestor_create,
            0,
        ))
        .unwrap_err(),
        Error::Denied,
        "a file-scoped grant cannot mutate an ancestor"
    );

    let before = (f.disk.reads, f.disk.writes, f.disk.flushes);
    let mut denied = Packet::new(CREATE);
    denied.id = f.directory;
    denied.count = 3;
    denied.data[..3].copy_from_slice(b"den");
    assert_eq!(
        result(send(&mut server, &mut f.disk, 1, reader.context, denied, 0)).unwrap_err(),
        Error::Denied
    );
    assert_eq!((f.disk.reads, f.disk.writes, f.disk.flushes), before);

    let mut remove = Packet::new(REMOVE);
    remove.id = f.directory;
    assert_eq!(
        result(send(&mut server, &mut f.disk, 0, writer.context, remove, 0)).unwrap_err(),
        Error::NotEmpty
    );
    for id in [new_file, new_dir.id] {
        let mut remove = Packet::new(REMOVE);
        remove.id = id;
        result(send(&mut server, &mut f.disk, 0, writer.context, remove, 0)).unwrap();
        assert_eq!(server.volume().stat(id), Err(FsError::NotFound));
    }
}

#[test]
fn plain_replacement_is_bounded_and_reads_old_bytes_until_commit() {
    let mut f = fixture(b"old bytes");
    let mut server = Server7::new(&mut f.volume);
    let writer = grant(&mut server, 0, f.workspace, TRACKED_WRITE7, 0);
    let start_version = version(&server, f.file);

    begin(
        &mut server,
        &mut f.disk,
        0,
        writer.context,
        f.file,
        0,
        start_version,
    )
    .unwrap();
    assert_eq!(
        read_all(&mut server, &mut f.disk, 0, writer.context, f.file, 9),
        b"old bytes"
    );
    let empty = bare(&mut server, &mut f.disk, 0, writer.context, COMMIT, f.file).unwrap();
    assert_eq!(
        (empty.id, empty.arg, empty.version, empty.count),
        (f.file, 0, version(&server, f.file), 40)
    );
    assert!(read_all(&mut server, &mut f.disk, 0, writer.context, f.file, 0).is_empty());

    let size_before = (f.disk.writes, f.disk.flushes);
    let empty_version = version(&server, f.file);
    assert_eq!(
        begin(
            &mut server,
            &mut f.disk,
            0,
            writer.context,
            f.file,
            MAX_INLINE + 1,
            empty_version,
        ),
        Err(Error::Size)
    );
    assert_eq!((f.disk.writes, f.disk.flushes), size_before);

    let bytes: Vec<u8> = (0..MAX_INLINE).map(|index| (index * 13) as u8).collect();
    let empty_version = version(&server, f.file);
    begin(
        &mut server,
        &mut f.disk,
        0,
        writer.context,
        f.file,
        bytes.len(),
        empty_version,
    )
    .unwrap();
    chunks(
        &mut server,
        &mut f.disk,
        0,
        writer.context,
        CHUNK,
        f.file,
        &bytes,
    )
    .unwrap();
    let committed = bare(&mut server, &mut f.disk, 0, writer.context, COMMIT, f.file).unwrap();
    assert_eq!(
        (committed.id, committed.arg as usize, committed.count),
        (f.file, MAX_INLINE, 40)
    );
    assert_eq!(
        read_all(
            &mut server,
            &mut f.disk,
            0,
            writer.context,
            f.file,
            MAX_INLINE
        ),
        bytes
    );
}

#[test]
fn wrong_object_and_offsets_do_not_discard_candidates_and_stale_commit_is_consumed() {
    let mut f = fixture(b"old");
    let mut server = Server7::new(&mut f.volume);
    let first = grant(&mut server, 0, f.workspace, TRACKED_WRITE7, 0);
    let second = grant(&mut server, 1, f.workspace, TRACKED_WRITE7, 0);
    let original_version = version(&server, f.file);
    begin(
        &mut server,
        &mut f.disk,
        0,
        first.context,
        f.file,
        5,
        original_version,
    )
    .unwrap();

    assert_eq!(
        chunk(
            &mut server,
            &mut f.disk,
            0,
            first.context,
            CHUNK,
            f.sibling,
            0,
            b"x"
        ),
        Err(Error::NoTransfer)
    );
    assert_eq!(
        chunk(
            &mut server,
            &mut f.disk,
            0,
            first.context,
            CHUNK,
            f.file,
            1,
            b"x"
        ),
        Err(Error::Offset)
    );
    assert_eq!(
        result(send(
            &mut server,
            &mut f.disk,
            0,
            first.context,
            Packet {
                id: f.file,
                ..Packet::new(COMMIT)
            },
            0,
        ))
        .unwrap_err(),
        Error::Offset,
        "an incomplete commit retains its candidate"
    );
    chunks(
        &mut server,
        &mut f.disk,
        0,
        first.context,
        CHUNK,
        f.file,
        b"first",
    )
    .unwrap();

    begin(
        &mut server,
        &mut f.disk,
        1,
        second.context,
        f.file,
        6,
        original_version,
    )
    .unwrap();
    chunks(
        &mut server,
        &mut f.disk,
        1,
        second.context,
        CHUNK,
        f.file,
        b"second",
    )
    .unwrap();
    bare(&mut server, &mut f.disk, 1, second.context, COMMIT, f.file).unwrap();
    assert_eq!(
        bare(&mut server, &mut f.disk, 0, first.context, COMMIT, f.file),
        Err(Error::Version)
    );
    assert_eq!(
        bare(&mut server, &mut f.disk, 0, first.context, COMMIT, f.file),
        Err(Error::NoTransfer),
        "a stale commit consumed its candidate"
    );
}

#[test]
fn plain_and_profile_transfers_share_two_slots_and_abort_only_their_own_kind() {
    let mut f = fixture(b"original");
    let mut server = Server7::new(&mut f.volume);
    let plain = grant(&mut server, 0, f.workspace, TRACKED_WRITE7, 0);
    let tracked = grant(&mut server, 1, f.workspace, TRACKED_WRITE7, 0);
    let third = grant(&mut server, 2, f.workspace, TRACKED_WRITE7, 0);
    let base_version = version(&server, f.file);
    begin(
        &mut server,
        &mut f.disk,
        0,
        plain.context,
        f.file,
        1,
        base_version,
    )
    .unwrap();

    for op in [rustic_abi::files::REPLACE_OPEN, admission::OPEN] {
        let open = profile_open(
            replacement(
                server.volume(),
                f.workspace,
                f.file,
                base_version,
                u64::from(op),
            ),
            plain.context,
            op,
        );
        assert_eq!(
            result(send(&mut server, &mut f.disk, 0, plain.context, open, 0)).unwrap_err(),
            Error::Busy
        );
    }

    let request = replacement(server.volume(), f.workspace, f.file, base_version, 77);
    let open = profile_open(request, tracked.context, rustic_abi::files::REPLACE_OPEN);
    result(send(&mut server, &mut f.disk, 1, tracked.context, open, 0)).unwrap();
    assert_eq!(
        begin(
            &mut server,
            &mut f.disk,
            1,
            tracked.context,
            f.file,
            1,
            base_version
        ),
        Err(Error::Busy),
        "BEGIN cannot overwrite this client's profile-2 stage"
    );
    assert_eq!(
        begin(
            &mut server,
            &mut f.disk,
            2,
            third.context,
            f.file,
            1,
            base_version
        ),
        Err(Error::Busy),
        "a third ordinary candidate exceeds the combined two-transfer bound"
    );
    let generic_abort = bare(
        &mut server,
        &mut f.disk,
        1,
        tracked.context,
        rustic_abi::files::ABORT,
        f.file,
    );
    assert_eq!(generic_abort, Err(Error::NoTransfer));
    let mut tracked_abort = Packet::new(rustic_abi::files::REPLACE_ABORT);
    tracked_abort.id = f.file;
    result(send(
        &mut server,
        &mut f.disk,
        1,
        tracked.context,
        tracked_abort,
        0,
    ))
    .unwrap();

    let admission_request = replacement(server.volume(), f.workspace, f.file, base_version, 78);
    let open = profile_open(admission_request, tracked.context, admission::OPEN);
    result(send(&mut server, &mut f.disk, 1, tracked.context, open, 0)).unwrap();
    let mut plain_begin = Packet::new(BEGIN);
    plain_begin.id = f.file;
    plain_begin.arg = 1;
    plain_begin.version = base_version;
    assert_eq!(
        result(send(
            &mut server,
            &mut f.disk,
            1,
            tracked.context,
            plain_begin,
            0
        ))
        .unwrap_err(),
        Error::Busy
    );
    assert_eq!(
        begin(
            &mut server,
            &mut f.disk,
            2,
            third.context,
            f.file,
            1,
            base_version
        ),
        Err(Error::Busy)
    );
    let admission_open = profile_open(
        replacement(server.volume(), f.workspace, f.file, base_version, 79),
        third.context,
        admission::OPEN,
    );
    assert_eq!(
        result(send(
            &mut server,
            &mut f.disk,
            2,
            third.context,
            admission_open,
            0,
        ))
        .unwrap_err(),
        Error::Busy,
        "admission OPEN shares the same global two-transfer limit"
    );
    assert_eq!(
        bare(
            &mut server,
            &mut f.disk,
            1,
            tracked.context,
            rustic_abi::files::ABORT,
            f.file
        ),
        Err(Error::NoTransfer)
    );
    let mut admission_abort = Packet::new(admission::ABORT);
    admission_abort.id = f.file;
    result(send(
        &mut server,
        &mut f.disk,
        1,
        tracked.context,
        admission_abort,
        0,
    ))
    .unwrap();

    chunk(
        &mut server,
        &mut f.disk,
        0,
        plain.context,
        CHUNK,
        f.file,
        0,
        b"p",
    )
    .unwrap();
    bare(&mut server, &mut f.disk, 0, plain.context, COMMIT, f.file).unwrap();
    assert_eq!(
        read_all(&mut server, &mut f.disk, 0, plain.context, f.file, 1),
        b"p"
    );
}

#[test]
fn authority_lifecycle_clears_plain_candidates_and_maintenance_observes_them() {
    for lifecycle in 0..4 {
        let mut f = fixture(b"seed");
        let mut server = Server7::new(&mut f.volume);
        let old = grant(
            &mut server,
            0,
            f.workspace,
            TRACKED_WRITE7,
            if lifecycle == 3 { 5 } else { 0 },
        );
        let file_version = version(&server, f.file);
        begin(
            &mut server,
            &mut f.disk,
            0,
            old.context,
            f.file,
            3,
            file_version,
        )
        .unwrap();
        assert_eq!(server.maintain_retention(&mut f.disk), Err(Error::Busy));

        match lifecycle {
            0 => {
                let replacement = grant(&mut server, 0, f.workspace, TRACKED_WRITE7, 0);
                assert_ne!(old.context, replacement.context);
            }
            1 => {
                server.revoke(0).unwrap();
                grant(&mut server, 0, f.workspace, TRACKED_WRITE7, 0);
            }
            2 => {
                server.detach(0);
                grant(&mut server, 0, f.workspace, TRACKED_WRITE7, 0);
            }
            _ => {
                assert_eq!(server.expire(5), 1);
                grant(&mut server, 0, f.workspace, TRACKED_WRITE7, 0);
            }
        }
        let current = server.grant_at(0).unwrap();
        assert_ne!(old.context, current.context);
        let peer = grant(&mut server, 1, f.workspace, TRACKED_WRITE7, 0);
        let third = grant(&mut server, 2, f.workspace, TRACKED_WRITE7, 0);
        let file_version = version(&server, f.file);
        begin(
            &mut server,
            &mut f.disk,
            0,
            current.context,
            f.file,
            1,
            file_version,
        )
        .unwrap();
        begin(
            &mut server,
            &mut f.disk,
            1,
            peer.context,
            f.file,
            1,
            file_version,
        )
        .unwrap();
        assert_eq!(
            begin(
                &mut server,
                &mut f.disk,
                2,
                third.context,
                f.file,
                1,
                file_version
            ),
            Err(Error::Busy)
        );
    }
}

#[test]
fn ordinary_replacements_continue_with_all_eight_retained_snapshots() {
    let mut f = fixture(b"initial snapshot");
    let header = *f.volume.header().unwrap();
    let instance = header.sequence + 1;
    let mut retained = Vec::new();
    let mut current = f.volume.stat(f.file).unwrap().version;
    for key in 1..=8 {
        let bytes = vec![key as u8; 80];
        let record = f
            .volume
            .replace_tracked(
                &mut f.disk,
                WriteIdentity7 {
                    subject: SUBJECT,
                    workspace: f.workspace,
                    object: f.file,
                    instance,
                    retry_epoch: header.epoch,
                    retry_key: key,
                },
                current,
                &bytes,
            )
            .unwrap();
        current = f.volume.stat(f.file).unwrap().version;
        retained.push((record, bytes));
    }
    assert_eq!(
        f.volume
            .retained_records()
            .unwrap()
            .iter()
            .flatten()
            .count(),
        8
    );
    let retained_records = *f.volume.retained_records().unwrap();
    let mut snapshot_before = vec![0; retained[0].1.len()];
    assert_eq!(
        f.volume
            .read_retained_range(&mut f.disk, &retained[0].0, 0, &mut snapshot_before)
            .unwrap(),
        snapshot_before.len()
    );
    assert_eq!(snapshot_before, retained[0].1);

    let mut server = Server7::new(&mut f.volume);
    let writer = grant(&mut server, 0, f.workspace, TRACKED_WRITE7, 0);
    for index in 0..9u8 {
        let version = version(&server, f.file);
        let bytes = vec![0xa0 + index; 80];
        begin(
            &mut server,
            &mut f.disk,
            0,
            writer.context,
            f.file,
            bytes.len(),
            version,
        )
        .unwrap();
        chunks(
            &mut server,
            &mut f.disk,
            0,
            writer.context,
            CHUNK,
            f.file,
            &bytes,
        )
        .unwrap();
        bare(&mut server, &mut f.disk, 0, writer.context, COMMIT, f.file).unwrap();
        assert_eq!(
            read_all(&mut server, &mut f.disk, 0, writer.context, f.file, 80),
            bytes
        );
    }
    assert_eq!(
        *server.volume().retained_records().unwrap(),
        retained_records
    );
    let mut snapshot_after = vec![0; retained[0].1.len()];
    assert_eq!(
        server
            .volume()
            .read_retained_range(&mut f.disk, &retained[0].0, 0, &mut snapshot_after)
            .unwrap(),
        snapshot_after.len()
    );
    assert_eq!(snapshot_after, snapshot_before);
}

#[test]
fn a_removed_target_can_be_aborted_by_its_bound_client_without_stranding_capacity() {
    let mut f = fixture(b"seed");
    let mut server = Server7::new(&mut f.volume);
    let writer = grant(&mut server, 0, f.workspace, TRACKED_WRITE7, 0);
    let remover = grant(&mut server, 1, f.workspace, TRACKED_WRITE7, 0);
    let expected = version(&server, f.file);
    begin(
        &mut server,
        &mut f.disk,
        0,
        writer.context,
        f.file,
        1,
        expected,
    )
    .unwrap();
    chunk(
        &mut server,
        &mut f.disk,
        0,
        writer.context,
        CHUNK,
        f.file,
        0,
        b"x",
    )
    .unwrap();
    bare(&mut server, &mut f.disk, 1, remover.context, REMOVE, f.file).unwrap();
    assert_eq!(
        bare(&mut server, &mut f.disk, 0, writer.context, COMMIT, f.file).unwrap_err(),
        Error::Denied
    );
    assert_eq!(
        server.maintain_retention(&mut f.disk).unwrap_err(),
        Error::Busy
    );
    let before = (f.disk.reads, f.disk.writes, f.disk.flushes);
    assert_eq!(
        bare(
            &mut server,
            &mut f.disk,
            1,
            remover.context,
            rustic_abi::files::ABORT,
            f.file
        )
        .unwrap_err(),
        Error::NoTransfer
    );
    assert_eq!(
        bare(
            &mut server,
            &mut f.disk,
            0,
            writer.context,
            rustic_abi::files::ABORT,
            f.sibling
        )
        .unwrap_err(),
        Error::NoTransfer
    );
    bare(
        &mut server,
        &mut f.disk,
        0,
        writer.context,
        rustic_abi::files::ABORT,
        f.file,
    )
    .unwrap();
    assert_eq!((f.disk.reads, f.disk.writes, f.disk.flushes), before);
    let expected = version(&server, f.sibling);
    begin(
        &mut server,
        &mut f.disk,
        0,
        writer.context,
        f.sibling,
        0,
        expected,
    )
    .unwrap();
    bare(
        &mut server,
        &mut f.disk,
        0,
        writer.context,
        COMMIT,
        f.sibling,
    )
    .unwrap();
    server.maintain_retention(&mut f.disk).unwrap();
}

#[test]
fn malformed_plain_packets_return_zero_error_replies_without_disk_io() {
    let mut f = fixture(b"seed");
    let mut server = Server7::new(&mut f.volume);
    let writer = grant(&mut server, 0, f.workspace, TRACKED_WRITE7, 0);
    let before = (f.disk.reads, f.disk.writes, f.disk.flushes);

    let mut malformed_begin = Packet::new(BEGIN);
    malformed_begin.id = f.file;
    malformed_begin.arg = 1;
    malformed_begin.version = version(&server, f.file);
    malformed_begin.count = 1;
    malformed_begin.data[0] = 0xaa;
    assert_eq!(
        result(send(
            &mut server,
            &mut f.disk,
            0,
            writer.context,
            malformed_begin,
            0
        ))
        .unwrap_err(),
        Error::Protocol
    );

    let mut malformed_chunk = Packet::new(CHUNK);
    malformed_chunk.id = f.file;
    malformed_chunk.count = 1;
    malformed_chunk.version = 1;
    malformed_chunk.data[0] = 0xbb;
    assert_eq!(
        result(send(
            &mut server,
            &mut f.disk,
            0,
            writer.context,
            malformed_chunk,
            0
        ))
        .unwrap_err(),
        Error::Protocol
    );
    assert_eq!((f.disk.reads, f.disk.writes, f.disk.flushes), before);
}
