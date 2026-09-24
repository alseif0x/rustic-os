// SPDX-License-Identifier: Apache-2.0
//! Profile-2 staged admission through the V7 service: streamed acceptance,
//! status and observation, explicit execution with its completion receipt,
//! version refusal, requested cancellation, exact retries without writes,
//! remount and the owner's maintenance refusal while an admission is open.
#[path = "v7_admission/authority.rs"]
mod authority;

use rustic_abi::files::{
    DATA, Error, OPERATION_PART, Packet, REPLACE_CHUNK, REPLACE_COMMIT,
    admission::{self as a, AdmissionId, ObservationV2, PreventionReason, State, Status},
    operation::{self, Key, OperationId, Retry},
    reference::{Epoch, References, Version, Workspace},
    workspace::{Lookup, Operation, RECEIPT_BYTES, Replacement},
};
use rustic_file_service::{ADMISSION7, GrantRequest7, Server7, TRACKED_WRITE7};
use rustic_fs::{Disk, Error as FsError, Kind, Volume7, format7::RecordState};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const LINEAGE: [u8; 16] = [0x7c; 16];
const PEER: u64 = 12;
const SUBJECT: u64 = 2;
const SIZE: usize = 64 * 1024 + 3;

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

impl Sparse {
    fn mutations(&self) -> (usize, usize) {
        (self.writes, self.flushes)
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

fn remount(disk: &mut Sparse) -> Volume7 {
    let mut volume = Volume7::EMPTY;
    volume.mount_into(disk).unwrap();
    volume
}

fn pattern(seed: u8, size: usize) -> Vec<u8> {
    (0..size)
        .map(|index| {
            seed.wrapping_mul(31)
                .wrapping_add((index * 7 + index / 509) as u8)
        })
        .collect()
}

fn version(volume: &Volume7, id: u32) -> u64 {
    volume.node(id).unwrap().unwrap().version
}

fn request(volume: &Volume7, workspace: u32, object: u32, key: u64) -> operation::Replacement {
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

fn status(p: Packet) -> Result<Packet, Error> {
    if p.status == 0 {
        return Ok(p);
    }
    assert_eq!((p.id, p.arg, p.version, p.count), (0, 0, 0, 0));
    Err(Error::parse(p.status).unwrap_err())
}

/// One client binding of the V7 service.
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
                    endpoint: 90 + slot as u64,
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

    fn send(&self, server: &mut Server7<'_>, disk: &mut Sparse, mut p: Packet) -> Packet {
        p.context = self.context;
        let reply = server.handle(disk, self.slot, PEER, p, 0);
        assert_eq!(reply.op, p.op);
        assert_eq!(reply.context, self.context);
        reply
    }

    /// Profile-2 admission OPEN followed by every CHUNK of `bytes`.
    fn stage(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        request: operation::Replacement,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let mut open = Replacement { request }
            .packet(bytes.len(), self.context)
            .unwrap();
        open.op = a::OPEN;
        let ack = status(self.send(server, disk, open))?;
        assert_eq!((ack.id, ack.arg, ack.version, ack.count), (0, 0, 0, 0));
        self.chunks(server, disk, a::CHUNK, request.resource.object(), bytes)
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
            let mut p = Packet::new(op);
            p.id = object;
            p.arg = (index * DATA) as u32;
            p.count = chunk.len() as u8;
            p.data[..chunk.len()].copy_from_slice(chunk);
            status(self.send(server, disk, p))?;
        }
        Ok(())
    }

    /// ACCEPT, COMMIT or ABORT naming only the object.
    fn bare(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        op: u8,
        object: u32,
    ) -> Result<Packet, Error> {
        let mut p = Packet::new(op);
        p.id = object;
        status(self.send(server, disk, p))
    }

    fn admit(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        request: operation::Replacement,
        bytes: &[u8],
    ) -> Result<Status, Error> {
        self.stage(server, disk, request, bytes)?;
        let reply = self.bare(server, disk, a::ACCEPT, request.resource.object())?;
        Ok(Status::decode(&reply).unwrap())
    }

    /// GET, EXECUTE or CANCEL of one admission ID.
    fn action(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        op: u8,
        id: AdmissionId,
    ) -> Result<Status, Error> {
        let reply = status(self.send(server, disk, id.packet(op, self.context).unwrap()))?;
        let result = Status::decode(&reply).unwrap();
        assert_eq!(result.id, id);
        Ok(result)
    }

    fn retry(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        workspace: Workspace,
        retry: Retry,
    ) -> Result<Status, Error> {
        let mut p = Lookup {
            query: operation::Lookup::Retry { workspace, retry },
        }
        .packet(self.context);
        p.op = a::RETRY;
        Ok(Status::decode(&status(self.send(server, disk, p))?).unwrap())
    }

    fn observe(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        id: AdmissionId,
    ) -> Result<ObservationV2, Error> {
        let p = ObservationV2::request(id, self.context).unwrap();
        Ok(ObservationV2::decode(&status(self.send(server, disk, p))?).unwrap())
    }

    /// Profile-2 receipt lookup by operation ID, with its later parts.
    fn receipt(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        id: OperationId,
    ) -> Result<Operation, Error> {
        let query = operation::Lookup::Id(id);
        let first = status(self.send(server, disk, Lookup { query }.packet(self.context)))?;
        let mut bytes = [0; RECEIPT_BYTES];
        for offset in [0usize, 40, 80] {
            let part = if offset == 0 {
                first
            } else {
                let mut p = Lookup { query }.packet(self.context);
                p.op = OPERATION_PART;
                p.arg = offset as u32;
                status(self.send(server, disk, p))?
            };
            let length = (RECEIPT_BYTES - offset).min(DATA);
            bytes[offset..offset + length].copy_from_slice(part.payload());
        }
        Ok(Operation::decode(&bytes).unwrap())
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn retained_state(volume: &Volume7, key: u64) -> Option<RecordState> {
    volume
        .retained_records()
        .unwrap()
        .iter()
        .flatten()
        .find(|record| record.retry_key == key)
        .map(|record| record.state)
}

#[test]
fn an_accepted_admission_is_durable_and_admitted_without_changing_the_file() {
    let mut f = fixture();
    let before = version(&f.volume, f.file);
    let admission = request(&f.volume, f.workspace, f.file, 0x51);
    let sequence = f.volume.header().unwrap().sequence;
    let bytes = pattern(5, SIZE);
    let mut server = Server7::new(&mut f.volume);
    let client = Client::grant(&mut server, 0, f.workspace, ADMISSION7, SUBJECT);
    let accepted = client
        .admit(&mut server, &mut f.disk, admission, &bytes)
        .unwrap();
    assert_eq!(accepted.state, State::Admitted);
    assert_eq!(
        accepted.id,
        AdmissionId::new(LINEAGE, sequence + 1).unwrap()
    );
    assert_eq!(accepted.service_instance.sequence(), sequence + 1);
    assert_eq!(accepted.terminal, 0);
    assert_eq!(accepted.completion(), None);
    assert_eq!(server.volume().open_stages(), 0);
    assert_eq!(version(server.volume(), f.file), before);

    let get = client
        .action(&mut server, &mut f.disk, a::GET, accepted.id)
        .unwrap();
    let retry = client
        .retry(
            &mut server,
            &mut f.disk,
            admission.workspace,
            admission.retry,
        )
        .unwrap();
    assert_eq!((get, retry), (accepted, accepted));
    assert_eq!(
        client
            .observe(&mut server, &mut f.disk, accepted.id)
            .unwrap(),
        ObservationV2::Retained {
            status: accepted,
            prevention: None
        }
    );
    // A tracked receipt lookup of the pending admission is `Busy`.
    let lookup = Lookup {
        query: operation::Lookup::Retry {
            workspace: admission.workspace,
            retry: admission.retry,
        },
    }
    .packet(client.context);
    assert_eq!(
        status(client.send(&mut server, &mut f.disk, lookup)),
        Err(Error::Busy)
    );
    drop(server);
    assert_eq!(retained_state(&f.volume, 0x51), Some(RecordState::Admitted));
}

#[test]
fn execute_commits_the_admitted_bytes_and_its_completion_receipt_matches() {
    let mut f = fixture();
    let previous = version(&f.volume, f.file);
    let admission = request(&f.volume, f.workspace, f.file, 0x52);
    let bytes = pattern(6, SIZE);
    let mut server = Server7::new(&mut f.volume);
    let client = Client::grant(&mut server, 0, f.workspace, ADMISSION7, SUBJECT);
    let accepted = client
        .admit(&mut server, &mut f.disk, admission, &bytes)
        .unwrap();
    let executed = client
        .action(&mut server, &mut f.disk, a::EXECUTE, accepted.id)
        .unwrap();
    assert_eq!(executed.state, State::Committed);
    assert_eq!(executed.service_instance, accepted.service_instance);
    assert!(executed.terminal > accepted.id.number());
    let completion = executed.completion().unwrap();
    assert_eq!(version(server.volume(), f.file), completion.sequence());

    let receipt = client
        .receipt(&mut server, &mut f.disk, completion)
        .unwrap();
    assert_eq!(receipt.id, completion);
    assert_eq!(receipt.workspace, admission.workspace);
    assert_eq!(receipt.resource, admission.resource);
    assert_eq!(receipt.retry, admission.retry);
    assert_eq!(receipt.previous_version.value(), previous);
    assert_eq!(receipt.version.value(), completion.sequence());
    assert_eq!(receipt.size as usize, bytes.len());
    assert_eq!(receipt.sha256, sha256(&bytes));

    // Terminal replays write nothing: execute again, a late cancel, GET.
    let quiet = f.disk.mutations();
    for op in [a::EXECUTE, a::CANCEL, a::GET] {
        assert_eq!(
            client
                .action(&mut server, &mut f.disk, op, accepted.id)
                .unwrap(),
            executed,
            "op {op}"
        );
    }
    assert_eq!(f.disk.mutations(), quiet);
    drop(server);
    assert_eq!(read_all(&f.volume, &mut f.disk, f.file), bytes);
    assert_eq!(
        retained_state(&f.volume, 0x52),
        Some(RecordState::AdmittedCommitted)
    );
}

#[test]
fn a_concurrent_write_refuses_execution_with_version_until_an_explicit_cancel() {
    let mut f = fixture();
    let admission = request(&f.volume, f.workspace, f.file, 0x53);
    let mut server = Server7::new(&mut f.volume);
    let client = Client::grant(&mut server, 0, f.workspace, ADMISSION7, SUBJECT);
    let accepted = client
        .admit(&mut server, &mut f.disk, admission, &pattern(7, 4000))
        .unwrap();

    // A tracked write under another key moves the file past the admission.
    let tracked = request(server.volume(), f.workspace, f.file, 0x54);
    let interloper = pattern(8, 700);
    client
        .stage_tracked(&mut server, &mut f.disk, tracked, &interloper)
        .unwrap();
    client
        .bare(&mut server, &mut f.disk, REPLACE_COMMIT, f.file)
        .unwrap();
    let moved = version(server.volume(), f.file);

    let quiet = f.disk.mutations();
    assert_eq!(
        client.action(&mut server, &mut f.disk, a::EXECUTE, accepted.id),
        Err(Error::Version)
    );
    assert_eq!(f.disk.mutations(), quiet);
    assert_eq!(
        client
            .action(&mut server, &mut f.disk, a::GET, accepted.id)
            .unwrap(),
        accepted
    );
    assert_eq!(version(server.volume(), f.file), moved);

    let cancelled = client
        .action(&mut server, &mut f.disk, a::CANCEL, accepted.id)
        .unwrap();
    assert_eq!(cancelled.state, State::Cancelled);
    assert!(cancelled.terminal > accepted.id.number());
    assert_eq!(cancelled.completion(), None);
    assert_eq!(
        client
            .observe(&mut server, &mut f.disk, accepted.id)
            .unwrap(),
        ObservationV2::Retained {
            status: cancelled,
            prevention: Some(PreventionReason::Requested)
        }
    );
    let quiet = f.disk.mutations();
    for op in [a::CANCEL, a::EXECUTE, a::GET] {
        assert_eq!(
            client
                .action(&mut server, &mut f.disk, op, accepted.id)
                .unwrap(),
            cancelled,
            "op {op}"
        );
    }
    assert_eq!(
        client
            .retry(
                &mut server,
                &mut f.disk,
                admission.workspace,
                admission.retry
            )
            .unwrap(),
        cancelled
    );
    assert_eq!(f.disk.mutations(), quiet);
    drop(server);
    assert_eq!(read_all(&f.volume, &mut f.disk, f.file), interloper);
}

#[test]
fn an_exact_retry_of_the_admission_returns_the_same_admission_without_writes() {
    let mut f = fixture();
    let admission = request(&f.volume, f.workspace, f.file, 0x55);
    let bytes = pattern(9, SIZE);
    let mut server = Server7::new(&mut f.volume);
    let client = Client::grant(&mut server, 0, f.workspace, ADMISSION7, SUBJECT);
    let accepted = client
        .admit(&mut server, &mut f.disk, admission, &bytes)
        .unwrap();
    let quiet = f.disk.mutations();
    assert_eq!(
        client
            .admit(&mut server, &mut f.disk, admission, &bytes)
            .unwrap(),
        accepted
    );
    assert_eq!(f.disk.mutations(), quiet);

    let mut different = bytes.clone();
    *different.last_mut().unwrap() ^= 1;
    assert_eq!(
        client.admit(&mut server, &mut f.disk, admission, &different),
        Err(Error::IdempotencyConflict)
    );
    assert_eq!(
        client.admit(&mut server, &mut f.disk, admission, &bytes[..100]),
        Err(Error::IdempotencyConflict)
    );
    assert_eq!(server.volume().open_stages(), 0);
    assert_eq!(f.disk.mutations(), quiet);

    // After execution the same retry reports the committed admission.
    let executed = client
        .action(&mut server, &mut f.disk, a::EXECUTE, accepted.id)
        .unwrap();
    let quiet = f.disk.mutations();
    assert_eq!(
        client
            .admit(&mut server, &mut f.disk, admission, &bytes)
            .unwrap(),
        executed
    );
    assert_eq!(f.disk.mutations(), quiet);
}

#[test]
fn a_remounted_admission_stays_admitted_and_a_new_mount_executes_it() {
    let mut f = fixture();
    let admission = request(&f.volume, f.workspace, f.file, 0x56);
    let bytes = pattern(10, SIZE);
    let accepted = {
        let mut server = Server7::new(&mut f.volume);
        let client = Client::grant(&mut server, 0, f.workspace, ADMISSION7, SUBJECT);
        client
            .admit(&mut server, &mut f.disk, admission, &bytes)
            .unwrap()
    };
    let mut volume = remount(&mut f.disk);
    let mut server = Server7::new(&mut volume);
    let client = Client::grant(&mut server, 1, f.workspace, ADMISSION7, SUBJECT);
    assert_eq!(
        client
            .action(&mut server, &mut f.disk, a::GET, accepted.id)
            .unwrap(),
        accepted
    );
    // The exact retry presents the instance of the earlier mount.
    let quiet = f.disk.mutations();
    assert_eq!(
        client
            .admit(&mut server, &mut f.disk, admission, &bytes)
            .unwrap(),
        accepted
    );
    assert_eq!(f.disk.mutations(), quiet);
    let executed = client
        .action(&mut server, &mut f.disk, a::EXECUTE, accepted.id)
        .unwrap();
    assert_eq!(executed.state, State::Committed);
    assert_eq!(executed.service_instance, accepted.service_instance);
    let receipt = client
        .receipt(&mut server, &mut f.disk, executed.completion().unwrap())
        .unwrap();
    assert_eq!(receipt.sha256, sha256(&bytes));
}

#[test]
fn maintenance_is_busy_while_an_admission_is_unresolved() {
    let mut f = fixture();
    let admission = request(&f.volume, f.workspace, f.file, 0x57);
    let epoch = f.volume.header().unwrap().epoch;
    let mut server = Server7::new(&mut f.volume);
    let client = Client::grant(&mut server, 0, f.workspace, ADMISSION7, SUBJECT);
    let accepted = client
        .admit(&mut server, &mut f.disk, admission, &pattern(11, 2000))
        .unwrap();
    let quiet = f.disk.mutations();
    assert_eq!(server.maintain_retention(&mut f.disk), Err(Error::Busy));
    assert_eq!(f.disk.mutations(), quiet);
    assert_eq!(
        client
            .action(&mut server, &mut f.disk, a::GET, accepted.id)
            .unwrap(),
        accepted
    );
    client
        .action(&mut server, &mut f.disk, a::CANCEL, accepted.id)
        .unwrap();
    let done = server.maintain_retention(&mut f.disk).unwrap();
    assert_eq!((done.previous_epoch, done.epoch), (epoch, epoch + 1));
    assert_eq!(done.records, 1);
    assert_eq!(
        client.action(&mut server, &mut f.disk, a::GET, accepted.id),
        Err(Error::OutcomeUnknown)
    );
}

impl Client {
    /// Profile-2 tracked REPLACE_OPEN followed by every REPLACE_CHUNK.
    fn stage_tracked(
        &self,
        server: &mut Server7<'_>,
        disk: &mut Sparse,
        request: operation::Replacement,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let open = Replacement { request }
            .packet(bytes.len(), self.context)
            .unwrap();
        status(self.send(server, disk, open))?;
        self.chunks(
            server,
            disk,
            REPLACE_CHUNK,
            request.resource.object(),
            bytes,
        )
    }
}

#[test]
fn the_tracked_profile_without_cancel_still_admits_and_executes() {
    let mut f = fixture();
    let admission = request(&f.volume, f.workspace, f.file, 0x58);
    let mut server = Server7::new(&mut f.volume);
    let client = Client::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    let accepted = client
        .admit(&mut server, &mut f.disk, admission, &pattern(12, 900))
        .unwrap();
    let quiet = f.disk.mutations();
    assert_eq!(
        client.action(&mut server, &mut f.disk, a::CANCEL, accepted.id),
        Err(Error::Denied)
    );
    assert_eq!(f.disk.mutations(), quiet);
    assert_eq!(
        client
            .action(&mut server, &mut f.disk, a::EXECUTE, accepted.id)
            .unwrap()
            .state,
        State::Committed
    );
}
