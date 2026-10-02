// SPDX-License-Identifier: Apache-2.0
//! Existing markerless profile-1 tracked-write and admission wire forms over
//! the shared V7 storage owner.
use rustic_abi::files::{
    DATA, Error, OPERATION_PART, REPLACE_ABORT, REPLACE_CHUNK, REPLACE_COMMIT,
    admission::{self as a, State, Status},
    operation::{self, Key, OperationId, Retry},
    reference::{Epoch, References, Version},
    workspace, *,
};
use rustic_file_service::{ADMISSION7, GrantRequest7, READ_ONLY7, Server7, TRACKED_WRITE7};
use rustic_fs::{Disk, Error as FsError, Kind, Volume7};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const LINEAGE: [u8; 16] = [0x7d; 16];
const PEER: u64 = 71;
const SUBJECT: u64 = 5;

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
        .create(&mut disk, workspace.id, b"target.bin", Kind::File)
        .unwrap();
    let sibling = volume
        .create(&mut disk, workspace.id, b"sibling.bin", Kind::File)
        .unwrap();
    Fixture {
        volume,
        disk,
        workspace: workspace.id,
        file: file.id,
        sibling: sibling.id,
    }
}

fn remount(disk: &mut Sparse) -> Volume7 {
    let mut volume = Volume7::EMPTY;
    volume.mount_into(disk).unwrap();
    volume
}

fn version(volume: &Volume7, object: u32) -> u64 {
    volume.node(object).unwrap().unwrap().version
}

fn replacement(volume: &Volume7, workspace: u32, object: u32, key: u64) -> operation::Replacement {
    let references = References::new(LINEAGE, workspace, object).unwrap();
    operation::Replacement {
        workspace: references.workspace,
        resource: references.resource,
        expected_version: Version::new(version(volume, object)).unwrap(),
        retry: Retry {
            epoch: Epoch::new(volume.header().unwrap().epoch).unwrap(),
            key: Key::new(key).unwrap(),
        },
    }
}

fn pattern(seed: u8, size: usize) -> Vec<u8> {
    (0..size)
        .map(|index| {
            seed.wrapping_mul(31)
                .wrapping_add((index * 11 + index / 509) as u8)
        })
        .collect()
}

fn status(packet: Packet) -> Result<Packet, Error> {
    if packet.status == 0 {
        Ok(packet)
    } else {
        Err(Error::parse(packet.status).unwrap_err())
    }
}

fn read_all(volume: &Volume7, disk: &mut Sparse, object: u32) -> Vec<u8> {
    let node = *volume.node(object).unwrap().unwrap();
    let mut bytes = vec![0; node.length as usize];
    let mut offset = 0;
    while offset < bytes.len() {
        let end = (offset + 1024).min(bytes.len());
        offset += volume
            .read_range(disk, object, None, offset as u64, &mut bytes[offset..end])
            .unwrap();
    }
    bytes
}

struct Client {
    slot: usize,
    context: u32,
}

impl Client {
    fn grant(server: &mut Server7<'_>, slot: usize, scope: u32, rights: u8, subject: u64) -> Self {
        let grant = server
            .grant(
                slot,
                GrantRequest7 {
                    peer: PEER,
                    endpoint: 900 + slot as u64,
                    scope,
                    rights,
                    subject,
                    expires: 0,
                },
            )
            .unwrap();
        Self {
            slot,
            context: grant.context,
        }
    }

    fn send(&self, server: &mut Server7<'_>, disk: &mut Sparse, mut packet: Packet) -> Packet {
        packet.context = self.context;
        let reply = server.handle(disk, self.slot, PEER, packet, 0);
        assert_eq!((reply.op, reply.context), (packet.op, self.context));
        reply
    }

    fn chunks(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        op: u8,
        object: u32,
        bytes: &[u8],
    ) -> Result<(), Error> {
        for (index, chunk) in bytes.chunks(DATA).enumerate() {
            let mut packet = Packet::new(op);
            packet.id = object;
            packet.arg = (index * DATA) as u32;
            packet.count = chunk.len() as u8;
            packet.data[..chunk.len()].copy_from_slice(chunk);
            status(self.send(server, disk, packet))?;
        }
        Ok(())
    }

    fn bare(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        op: u8,
        object: u32,
    ) -> Result<Packet, Error> {
        let mut packet = Packet::new(op);
        packet.id = object;
        status(self.send(server, disk, packet))
    }

    fn admission_profile1(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        request: operation::Replacement,
        bytes: &[u8],
    ) -> Result<Status, Error> {
        let mut open = request.packet(bytes.len(), self.context)?;
        open.op = a::OPEN;
        status(self.send(server, disk, open))?;
        self.chunks(server, disk, a::CHUNK, request.resource.object(), bytes)?;
        let accepted = self.bare(server, disk, a::ACCEPT, request.resource.object())?;
        Status::decode(&accepted)
    }

    fn part(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        id: OperationId,
        offset: usize,
    ) -> Packet {
        let mut request = operation::Lookup::Id(id).packet(self.context);
        request.op = OPERATION_PART;
        request.arg = offset as u32;
        status(self.send(server, disk, request)).unwrap()
    }
}

fn legacy_receipt_parts(
    first: Packet,
    client: &Client,
    server: &mut Server7<'_>,
    disk: &mut Sparse,
) -> ([u8; operation::RECEIPT_BYTES], operation::Operation) {
    let id = OperationId::new(LINEAGE, first.version).unwrap();
    let mut bytes = [0; operation::RECEIPT_BYTES];
    for offset in [0usize, 40, 80] {
        let part = if offset == 0 {
            first
        } else {
            client.part(server, disk, id, offset)
        };
        let length = (operation::RECEIPT_BYTES - offset).min(DATA);
        assert_eq!(
            (part.id, part.arg, part.version, part.count as usize),
            (
                offset as u32,
                operation::RECEIPT_BYTES as u32,
                id.sequence(),
                length
            )
        );
        bytes[offset..offset + length].copy_from_slice(&part.data[..length]);
    }
    let receipt = operation::Operation::decode(&bytes).unwrap();
    assert_eq!(receipt.id, id);
    (bytes, receipt)
}

#[test]
fn profile1_commit_and_cold_id_retry_lookups_keep_exact_legacy_receipt_bytes() {
    let mut f = fixture();
    let bytes = pattern(4, 1024);
    let request = replacement(&f.volume, f.workspace, f.file, 0x101);
    let (committed_bytes, committed) = {
        let mut server = Server7::new(&mut f.volume);
        let client = Client::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
        let open = request.packet(bytes.len(), client.context).unwrap();
        assert_eq!(open.count, 36);
        status(client.send(&mut server, &mut f.disk, open)).unwrap();
        client
            .chunks(&mut server, &mut f.disk, REPLACE_CHUNK, f.file, &bytes)
            .unwrap();
        let first = client
            .bare(&mut server, &mut f.disk, REPLACE_COMMIT, f.file)
            .unwrap();
        legacy_receipt_parts(first, &client, &mut server, &mut f.disk)
    };
    assert_eq!(committed.size, 1024);
    assert_eq!(committed.sha256, <[u8; 32]>::from(Sha256::digest(&bytes)));
    assert_eq!(committed.encode().unwrap(), committed_bytes);

    let mut volume = remount(&mut f.disk);
    let mut server = Server7::new(&mut volume);
    let client = Client::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    for query in [
        operation::Lookup::Id(committed.id),
        operation::Lookup::Retry {
            workspace: committed.workspace,
            retry: committed.retry,
        },
    ] {
        let first =
            status(client.send(&mut server, &mut f.disk, query.packet(client.context))).unwrap();
        assert_eq!(
            first,
            committed.part(first.op, client.context, 0).unwrap(),
            "cold first part retains the markerless receipt bytes"
        );
        for offset in [40usize, 80] {
            assert_eq!(
                client.part(&mut server, &mut f.disk, committed.id, offset),
                committed
                    .part(OPERATION_PART, client.context, offset)
                    .unwrap()
            );
        }
    }
    drop(server);
    assert_eq!(read_all(&volume, &mut f.disk, f.file), bytes);
}

#[test]
fn profile1_size_limit_checks_authority_and_scope_before_any_snapshot_read() {
    let mut f = fixture();
    let bytes = pattern(5, 4096);
    let request = replacement(&f.volume, f.workspace, f.file, 0x102);
    let receipt_id = {
        let mut server = Server7::new(&mut f.volume);
        let client = Client::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
        let open = workspace::Replacement { request }
            .packet(bytes.len(), client.context)
            .unwrap();
        assert_eq!(open.count, 40);
        status(client.send(&mut server, &mut f.disk, open)).unwrap();
        client
            .chunks(&mut server, &mut f.disk, REPLACE_CHUNK, f.file, &bytes)
            .unwrap();
        OperationId::new(
            LINEAGE,
            client
                .bare(&mut server, &mut f.disk, REPLACE_COMMIT, f.file)
                .unwrap()
                .version,
        )
        .unwrap()
    };

    let mut volume = remount(&mut f.disk);
    let mut server = Server7::new(&mut volume);
    let owner = Client::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    let reads = f.disk.reads;
    let writes = (f.disk.writes, f.disk.flushes);
    for query in [
        operation::Lookup::Id(receipt_id),
        operation::Lookup::Retry {
            workspace: References::new(LINEAGE, f.workspace, f.file)
                .unwrap()
                .workspace,
            retry: request.retry,
        },
    ] {
        assert_eq!(
            status(owner.send(&mut server, &mut f.disk, query.packet(owner.context))),
            Err(Error::Size)
        );
    }
    assert_eq!(
        f.disk.reads, reads,
        "oversized profile-1 lookup skips digest I/O"
    );
    assert_eq!((f.disk.writes, f.disk.flushes), writes);
    assert_eq!(
        server.pending(),
        0,
        "the size refusal opens no transfer stage"
    );
    assert_eq!(server.volume().open_stages(), 0);

    let outside = Client::grant(&mut server, 1, f.sibling, TRACKED_WRITE7, SUBJECT);
    assert_eq!(
        status(outside.send(
            &mut server,
            &mut f.disk,
            operation::Lookup::Id(receipt_id).packet(outside.context),
        )),
        Err(Error::OutcomeUnknown),
        "scope filtering precedes the profile-1 size limit"
    );
    let no_inspect = Client::grant(&mut server, 2, f.workspace, READ_ONLY7, 0);
    assert_eq!(
        status(no_inspect.send(
            &mut server,
            &mut f.disk,
            operation::Lookup::Id(receipt_id).packet(no_inspect.context),
        )),
        Err(Error::Denied),
        "inspection authority precedes the profile-1 size limit"
    );
    assert_eq!(f.disk.reads, reads);
}

#[test]
fn malformed_profile2_marker_is_a_protocol_error_before_any_stage_or_disk_work() {
    let mut f = fixture();
    let request = replacement(&f.volume, f.workspace, f.file, 0x103);
    let mut server = Server7::new(&mut f.volume);
    let client = Client::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    let mut open = workspace::Replacement { request }
        .packet(3, client.context)
        .unwrap();
    open.data[36] = 3;
    let reads = f.disk.reads;
    let writes = (f.disk.writes, f.disk.flushes);
    assert_eq!(
        status(client.send(&mut server, &mut f.disk, open)),
        Err(Error::Protocol)
    );
    let mut lookup = workspace::Lookup {
        query: operation::Lookup::Id(OperationId::new(LINEAGE, 1).unwrap()),
    }
    .packet(client.context);
    lookup.data[16] = 3;
    assert_eq!(
        status(client.send(&mut server, &mut f.disk, lookup)),
        Err(Error::Protocol)
    );
    assert_eq!(server.volume().open_stages(), 0);
    assert_eq!(f.disk.reads, reads);
    assert_eq!((f.disk.writes, f.disk.flushes), writes);
}

#[test]
fn profile1_admission_open_retry_execute_and_cancel_share_status_semantics() {
    let mut f = fixture();
    let bytes = pattern(6, 777);
    let mut server = Server7::new(&mut f.volume);
    let client = Client::grant(&mut server, 0, f.workspace, ADMISSION7, SUBJECT);
    let admitted_request = replacement(server.volume(), f.workspace, f.file, 0x104);
    let accepted = client
        .admission_profile1(&mut server, &mut f.disk, admitted_request, &bytes)
        .unwrap();
    assert_eq!(accepted.state, State::Admitted);
    assert_eq!(server.volume().node(f.file).unwrap().unwrap().length, 0);

    let mut retry = operation::Lookup::Retry {
        workspace: admitted_request.workspace,
        retry: admitted_request.retry,
    }
    .packet(client.context);
    assert_eq!(retry.count, 24);
    retry.op = a::RETRY;
    assert_eq!(
        Status::decode(&status(client.send(&mut server, &mut f.disk, retry)).unwrap()).unwrap(),
        accepted
    );

    let execute = accepted.id.packet(a::EXECUTE, client.context).unwrap();
    let committed =
        Status::decode(&status(client.send(&mut server, &mut f.disk, execute)).unwrap()).unwrap();
    assert_eq!(committed.state, State::Committed);
    assert_eq!(read_all(server.volume(), &mut f.disk, f.file), bytes);

    let cancelled_request = replacement(server.volume(), f.workspace, f.file, 0x105);
    let candidate = pattern(7, 29);
    let pending = client
        .admission_profile1(&mut server, &mut f.disk, cancelled_request, &candidate)
        .unwrap();
    assert_eq!(pending.state, State::Admitted);
    let cancel = pending.id.packet(a::CANCEL, client.context).unwrap();
    let cancelled =
        Status::decode(&status(client.send(&mut server, &mut f.disk, cancel)).unwrap()).unwrap();
    assert_eq!(cancelled.state, State::Cancelled);
    assert_eq!(read_all(server.volume(), &mut f.disk, f.file), bytes);
}

#[test]
fn profile1_and_profile2_admission_and_tracked_stages_share_kind_and_capacity() {
    let mut f = fixture();
    let p1_admission = replacement(&f.volume, f.workspace, f.file, 0x106);
    let p2_tracked = replacement(&f.volume, f.workspace, f.file, 0x107);
    let p2_admission = replacement(&f.volume, f.workspace, f.file, 0x108);
    let mut server = Server7::new(&mut f.volume);
    let clients = [
        Client::grant(&mut server, 0, f.workspace, ADMISSION7, SUBJECT),
        Client::grant(&mut server, 1, f.workspace, ADMISSION7, SUBJECT),
        Client::grant(&mut server, 2, f.workspace, ADMISSION7, SUBJECT),
    ];

    let mut first = p1_admission.packet(1, clients[0].context).unwrap();
    first.op = a::OPEN;
    status(clients[0].send(&mut server, &mut f.disk, first)).unwrap();

    let second = workspace::Replacement {
        request: p2_tracked,
    }
    .packet(1, clients[1].context)
    .unwrap();
    status(clients[1].send(&mut server, &mut f.disk, second)).unwrap();

    let mut third = workspace::Replacement {
        request: p2_admission,
    }
    .packet(1, clients[2].context)
    .unwrap();
    third.op = a::OPEN;
    assert_eq!(
        status(clients[2].send(&mut server, &mut f.disk, third)),
        Err(Error::Busy),
        "the profile-1 admission and profile-2 tracked stage use the shared two-stage bound"
    );
    assert_eq!(server.volume().open_stages(), 2);

    assert_eq!(
        clients[0].bare(&mut server, &mut f.disk, REPLACE_COMMIT, f.file),
        Err(Error::NoTransfer),
        "tracked commit cannot finish a profile-1 admission stage"
    );
    assert_eq!(
        clients[1].bare(&mut server, &mut f.disk, a::ACCEPT, f.file),
        Err(Error::NoTransfer),
        "admission accept cannot finish a profile-2 tracked stage"
    );
    clients[0]
        .bare(&mut server, &mut f.disk, a::ABORT, f.file)
        .unwrap();
    clients[1]
        .bare(&mut server, &mut f.disk, REPLACE_ABORT, f.file)
        .unwrap();
    assert_eq!(server.volume().open_stages(), 0);
}
