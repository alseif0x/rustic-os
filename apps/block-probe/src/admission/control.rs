// SPDX-License-Identifier: Apache-2.0
//! Native V7 service-policy fixture using the existing admission wire packets.
use super::disk::Owned;
use crate::volume_disk::Disk;
use core::cell::Cell;
use rustic_file_service::{ADMISSION7, Control7, Grant7, GrantRequest7, Server7};
use rustic_fs::Publication7Phase;
use rustic_sdk::abi::files::{
    DATA, Error, INSPECT_RIGHT, Packet,
    admission::{self as a, AdmissionId, Status},
    operation, workspace,
};

const SLOT: usize = 0;
const PEER: u64 = 10;

pub(super) fn grant(s: &mut Server7<'_>, write: bool) -> Grant7 {
    grant_subject(s, write, 9)
}

pub(super) fn foreign(s: &mut Server7<'_>, write: bool) -> Grant7 {
    grant_subject(s, write, 8)
}

fn grant_subject(s: &mut Server7<'_>, write: bool, subject: u64) -> Grant7 {
    s.grant(
        SLOT,
        GrantRequest7 {
            peer: PEER,
            endpoint: 1,
            scope: 4,
            rights: if write { ADMISSION7 } else { INSPECT_RIGHT },
            subject,
            expires: 0,
        },
    )
    .unwrap()
}

fn good(packet: Packet) -> Packet {
    assert_eq!(packet.status, 0);
    packet
}

fn decode_error(status: u8) -> Error {
    match status {
        value if value == Error::Revoked as u8 => Error::Revoked,
        value if value == Error::Uncertain as u8 => Error::Uncertain,
        value if value == Error::Denied as u8 => Error::Denied,
        value if value == Error::OutcomeUnknown as u8 => Error::OutcomeUnknown,
        value if value == Error::IdempotencyConflict as u8 => Error::IdempotencyConflict,
        _ => panic!("unexpected admission error {status}"),
    }
}

fn checked(packet: Packet) -> Result<Packet, Error> {
    if packet.status == 0 {
        Ok(packet)
    } else {
        Err(decode_error(packet.status))
    }
}

fn status_reply(packet: Packet) -> Result<Status, Error> {
    Ok(Status::decode(&checked(packet)?).unwrap())
}

pub(super) fn admit_with_control(
    s: &mut Server7<'_>,
    disk: &mut Disk<'_>,
    caller: Grant7,
    request: operation::Replacement,
    bytes: &[u8],
    count: &Cell<usize>,
    mut control: impl FnMut(&mut Control7<'_>) -> u64,
) -> Result<Status, Error> {
    let mut open = workspace::Replacement { request }
        .packet(bytes.len(), caller.context)
        .unwrap();
    open.op = a::OPEN;
    checked(s.handle(disk, SLOT, PEER, open, 0))?;
    for (index, chunk) in bytes.chunks(DATA).enumerate() {
        let mut packet = Packet::new(a::CHUNK);
        packet.context = caller.context;
        packet.id = request.resource.object();
        packet.arg = (index * DATA) as u32;
        packet.count = chunk.len() as u8;
        packet.data[..chunk.len()].copy_from_slice(chunk);
        checked(s.handle(disk, SLOT, PEER, packet, 0))?;
    }
    let mut accept = Packet::new(a::ACCEPT);
    accept.context = caller.context;
    accept.id = request.resource.object();
    status_reply(s.handle_with(
        &mut Owned::new(disk, count),
        SLOT,
        PEER,
        accept,
        0,
        |owner| control(owner),
    ))
}

#[inline(never)]
pub(super) fn admit(
    s: &mut Server7<'_>,
    disk: &mut Disk<'_>,
    request: operation::Replacement,
    bytes: &[u8],
) -> Status {
    let caller = grant(s, true);
    let count = Cell::new(0);
    admit_with_control(s, disk, caller, request, bytes, &count, |_| 1).unwrap()
}

pub(super) fn status(
    s: &mut Server7<'_>,
    disk: &mut Disk<'_>,
    caller: Grant7,
    id: AdmissionId,
) -> Status {
    Status::decode(&good(s.handle(
        disk,
        SLOT,
        PEER,
        id.packet(a::GET, caller.context).unwrap(),
        0,
    )))
    .unwrap()
}

pub(super) fn try_status(
    s: &mut Server7<'_>,
    disk: &mut Disk<'_>,
    caller: Grant7,
    id: AdmissionId,
) -> Result<Status, Error> {
    status_reply(s.handle(
        disk,
        SLOT,
        PEER,
        id.packet(a::GET, caller.context).unwrap(),
        0,
    ))
}

pub(super) fn execute_with_control(
    s: &mut Server7<'_>,
    disk: &mut Disk<'_>,
    caller: Grant7,
    id: AdmissionId,
    count: &Cell<usize>,
    control: impl FnMut(&mut Control7<'_>) -> u64,
) -> Result<Status, Error> {
    status_reply(s.handle_with(
        &mut Owned::new(disk, count),
        SLOT,
        PEER,
        id.packet(a::EXECUTE, caller.context).unwrap(),
        0,
        control,
    ))
}

pub(super) fn cancel(
    s: &mut Server7<'_>,
    disk: &mut Disk<'_>,
    caller: Grant7,
    id: AdmissionId,
) -> Status {
    Status::decode(&good(s.handle(
        disk,
        SLOT,
        PEER,
        id.packet(a::CANCEL, caller.context).unwrap(),
        0,
    )))
    .unwrap()
}

#[inline(never)]
pub(super) fn seed(s: &mut Server7<'_>, disk: &mut Disk<'_>) {
    let caller = grant(s, true);
    let count = Cell::new(0);
    let request = super::request(s.volume(), 51);
    let mut waits = 0;
    let mut revoked = false;
    assert_eq!(
        admit_with_control(
            s,
            disk,
            caller,
            request,
            b"cancelled bytes",
            &count,
            |control| {
                if !revoked && control.pending() && control.phase() == Publication7Phase::Settling {
                    control.revoke(SLOT).unwrap();
                    revoked = true;
                }
                if control.pending() && revoked {
                    waits += 1;
                }
                1
            }
        ),
        Err(Error::Revoked)
    );
    assert!(waits >= 3);

    let admitted = admit(s, disk, super::request(s.volume(), 52), b"after");
    let caller = grant(s, true);
    count.set(0);
    waits = 0;
    revoked = false;
    assert_eq!(
        execute_with_control(s, disk, caller, admitted.id, &count, |control| {
            if !revoked && control.pending() && control.phase() == Publication7Phase::Settling {
                control.revoke(SLOT).unwrap();
                revoked = true;
            }
            if control.pending() && revoked {
                waits += 1;
            }
            1
        }),
        Err(Error::Uncertain)
    );
    assert!(waits >= 3);
}
