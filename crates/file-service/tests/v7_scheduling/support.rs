// SPDX-License-Identifier: Apache-2.0
use core::task::Poll;
use rustic_abi::files::{
    DATA, Error, Packet,
    admission::{self as a, Activity, ActivityPhase, AdmissionId, ObservationV2, State, Status},
    lifecycle::CancelAck,
    operation::{self, Key, Retry},
    reference::{Epoch, References, Version},
    workspace::Replacement,
};
use rustic_file_service::{ADMISSION7, Grant7, GrantRequest7, Server7};
use rustic_fs::{Disk, Error as FsError, Kind, PollDisk, PollDisk7, Volume7};
use std::{cell::Cell, collections::BTreeMap, rc::Rc};

pub const LINEAGE: [u8; 16] = [0x6d; 16];
pub const PEER: u64 = 12;
pub const SUBJECT: u64 = 9;

#[derive(Default)]
pub struct Sparse {
    pub sectors: BTreeMap<u64, [u8; 512]>,
}

impl Disk for Sparse {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), FsError> {
        *bytes = self.sectors.get(&sector).copied().unwrap_or([0; 512]);
        Ok(())
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), FsError> {
        self.sectors.insert(sector, *bytes);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), FsError> {
        Ok(())
    }
}

impl PollDisk for Sparse {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), FsError>> {
        Poll::Ready(self.write(sector, bytes))
    }

    fn poll_flush(&mut self) -> Poll<Result<(), FsError>> {
        Poll::Ready(self.flush())
    }
}

impl PollDisk7 for Sparse {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), FsError>> {
        Poll::Ready(self.read(sector, bytes))
    }
}

pub struct Fixture {
    pub volume: Volume7,
    pub disk: Sparse,
    pub workspace: u32,
}

pub fn fixture() -> Fixture {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    let workspace = volume
        .create(&mut disk, 4, b"alpha", Kind::Directory)
        .unwrap()
        .id;
    Fixture {
        volume,
        disk,
        workspace,
    }
}

pub fn remount(disk: &mut Sparse) -> Volume7 {
    let mut volume = Volume7::EMPTY;
    volume.mount_into(disk).unwrap();
    volume
}

pub fn grant(
    server: &mut Server7<'_>,
    slot: usize,
    scope: u32,
    rights: u8,
    subject: u64,
    expires: u64,
) -> Grant7 {
    server
        .grant(
            slot,
            GrantRequest7 {
                peer: PEER + slot as u64,
                endpoint: 90 + slot as u64,
                scope,
                rights,
                subject,
                expires,
            },
        )
        .unwrap()
}

pub fn request(volume: &Volume7, workspace: u32, object: u32, key: u64) -> operation::Replacement {
    let node = volume.node(object).unwrap().unwrap();
    let references = References::new(LINEAGE, workspace, object).unwrap();
    operation::Replacement {
        workspace: references.workspace,
        resource: references.resource,
        expected_version: Version::new(node.version).unwrap(),
        retry: Retry {
            epoch: Epoch::new(volume.header().unwrap().epoch).unwrap(),
            key: Key::new(key).unwrap(),
        },
    }
}

pub fn admit(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    grant: Grant7,
    workspace: u32,
    object: u32,
    key: u64,
    bytes: &[u8],
) -> Status {
    let request = request(server.volume(), workspace, object, key);
    let mut open = Replacement { request }
        .packet(bytes.len(), grant.context)
        .unwrap();
    open.op = a::OPEN;
    good(server.handle(disk, slot_for(grant), grant.peer, open, 0));
    for (index, chunk) in bytes.chunks(DATA).enumerate() {
        let mut p = Packet::new(a::CHUNK);
        p.context = grant.context;
        p.id = object;
        p.arg = (index * DATA) as u32;
        p.count = chunk.len() as u8;
        p.data[..chunk.len()].copy_from_slice(chunk);
        good(server.handle(disk, slot_for(grant), grant.peer, p, 0));
    }
    let mut accept = Packet::new(a::ACCEPT);
    accept.context = grant.context;
    accept.id = object;
    let reply = good(server.handle(disk, slot_for(grant), grant.peer, accept, 0));
    Status::decode(&reply).unwrap()
}

pub fn slot_for(grant: Grant7) -> usize {
    (grant.endpoint - 90) as usize
}

pub fn good(packet: Packet) -> Packet {
    if packet.status == 0 {
        packet
    } else {
        panic!("unexpected file-service error {}", packet.status)
    }
}

pub fn error(packet: Packet, expected: Error) {
    assert_eq!(packet.status, expected as u8);
    assert_eq!(
        (packet.id, packet.arg, packet.version, packet.count),
        (0, 0, 0, 0)
    );
    assert_eq!(packet.data, [0; DATA]);
}

pub fn send(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    grant: Grant7,
    op: u8,
    id: AdmissionId,
    now: u64,
) -> Packet {
    let p = id.packet(op, grant.context).unwrap();
    server.handle(disk, slot_for(grant), grant.peer, p, now)
}

pub fn lifecycle_cancel(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    grant: Grant7,
    id: AdmissionId,
    now: u64,
) -> CancelAck {
    let p = CancelAck::request(id, grant.context).unwrap();
    CancelAck::decode(&server.handle(disk, slot_for(grant), grant.peer, p, now)).unwrap()
}

pub fn observation(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    grant: Grant7,
    id: AdmissionId,
    now: u64,
) -> ObservationV2 {
    let p = ObservationV2::request(id, grant.context).unwrap();
    ObservationV2::decode(&server.handle(disk, slot_for(grant), grant.peer, p, now)).unwrap()
}

pub fn schedule(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    grant: Grant7,
    id: AdmissionId,
) -> Packet {
    send(server, disk, grant, a::SCHEDULE, id, 0)
}

pub fn admission_status(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    grant: Grant7,
    id: AdmissionId,
) -> Status {
    Status::decode(&good(send(server, disk, grant, a::GET, id, 0))).unwrap()
}

pub fn rights() -> u8 {
    ADMISSION7
}

#[derive(Default)]
pub struct PendingSignals {
    pub calls: Cell<usize>,
    pub release: Cell<bool>,
    pub fail: Cell<bool>,
}

enum Command {
    Write(u64, Box<[u8; 512]>),
    Flush,
}

pub struct Deferred {
    pub inner: Sparse,
    pub signals: Rc<PendingSignals>,
    pending: Option<Command>,
    hold_call: usize,
}

impl Deferred {
    pub fn new(inner: Sparse, hold_call: usize) -> Self {
        Self {
            inner,
            signals: Rc::new(PendingSignals::default()),
            pending: None,
            hold_call,
        }
    }

    fn complete(&mut self, command: Command) -> Poll<Result<(), FsError>> {
        if let Some(pending) = self.pending.take() {
            if !self.signals.release.get() {
                self.pending = Some(pending);
                return Poll::Pending;
            }
            if self.signals.fail.get() {
                return Poll::Ready(Err(FsError::Io));
            }
            return Poll::Ready(match pending {
                Command::Write(sector, bytes) => self.inner.write(sector, &bytes),
                Command::Flush => self.inner.flush(),
            });
        }
        let call = self.signals.calls.get() + 1;
        self.signals.calls.set(call);
        if call == self.hold_call {
            self.pending = Some(command);
            Poll::Pending
        } else if self.signals.fail.get() {
            Poll::Ready(Err(FsError::Io))
        } else {
            Poll::Ready(match command {
                Command::Write(sector, bytes) => self.inner.write(sector, &bytes),
                Command::Flush => self.inner.flush(),
            })
        }
    }
}

impl Disk for Deferred {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), FsError> {
        self.inner.read(sector, bytes)
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), FsError> {
        self.inner.write(sector, bytes)
    }
    fn flush(&mut self) -> Result<(), FsError> {
        self.inner.flush()
    }
}

impl PollDisk for Deferred {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), FsError>> {
        self.complete(Command::Write(sector, Box::new(*bytes)))
    }
    fn poll_flush(&mut self) -> Poll<Result<(), FsError>> {
        self.complete(Command::Flush)
    }
}

impl PollDisk7 for Deferred {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), FsError>> {
        Poll::Ready(self.inner.read(sector, bytes))
    }
}

pub fn assert_active(view: ObservationV2, phase: ActivityPhase, cancelled: bool, pending: bool) {
    let ObservationV2::Active(Activity {
        phase: observed,
        cancel_requested,
        io_pending,
        ..
    }) = view
    else {
        panic!("expected live activity");
    };
    assert_eq!(observed, phase);
    assert_eq!(cancel_requested, cancelled);
    assert_eq!(io_pending, pending);
}

pub fn assert_state(view: ObservationV2, state: State, reason: Option<a::PreventionReason>) {
    let ObservationV2::Retained { status, prevention } = view else {
        panic!("expected retained observation");
    };
    assert_eq!(status.state, state);
    assert_eq!(prevention, reason);
}

pub fn lifecycle_cancel_packet(id: AdmissionId, context: u32) -> Packet {
    CancelAck::request(id, context).unwrap()
}

pub fn request_cancel_packet(id: AdmissionId, context: u32) -> Packet {
    id.packet(a::REQUEST_CANCEL, context).unwrap()
}
