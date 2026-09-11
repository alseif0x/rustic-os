// SPDX-License-Identifier: Apache-2.0
//! Native service-policy fixture; private owner IPC is proved in recovery-test.
use super::{disk::Owned, request};
use crate::volume_disk::Disk;
use core::cell::Cell;
use rustic_file_service::{Caller, Grant, Server};
use rustic_fs::{AdmissionStatus, Replacement};
use rustic_sdk::abi::files::{Error, INSPECT_RIGHT, WRITE_RIGHT};

pub(super) fn grant(s: &mut Server, write: bool) -> Caller {
    let context = s
        .grant(
            0,
            Grant {
                peer: 10,
                endpoint: 1,
                scope: 4,
                rights: INSPECT_RIGHT | if write { WRITE_RIGHT } else { 0 },
                generation: 0,
                expires: 0,
                subject: 9,
            },
        )
        .unwrap();
    Caller {
        slot: 0,
        peer: 10,
        context,
    }
}

#[inline(never)]
pub(super) fn admit(
    s: &mut Server,
    disk: &mut Disk<'_>,
    r: Replacement,
    bytes: &[u8],
) -> AdmissionStatus {
    let caller = grant(s, true);
    let count = Cell::new(0);
    s.admit_with(&mut Owned::new(disk, &count), caller, r, bytes, |_, _| 1)
        .unwrap()
}

#[inline(never)]
pub(super) fn seed(s: &mut Server, disk: &mut Disk<'_>) {
    let caller = grant(s, true);
    let count = Cell::new(0);
    let r = request(&s.volume, 51);
    let mut waits = 0;
    assert_eq!(
        s.admit_with(
            &mut Owned::new(disk, &count),
            caller,
            r,
            b"cancelled bytes",
            |clients, pending| {
                if pending && count.get() == 13 {
                    clients.revoke(0).unwrap();
                    waits += 1;
                }
                1
            }
        ),
        Err(Error::Revoked)
    );
    assert!(waits >= 3);
    assert_eq!(count.get(), 28); // Late admission plus durable terminal cancellation.
    let r = request(&s.volume, 52);
    let b = admit(s, disk, r, b"after");
    let caller = grant(s, true);
    count.set(0);
    waits = 0;
    assert_eq!(
        s.execute_admission_with(
            &mut Owned::new(disk, &count),
            caller,
            b.id,
            |clients, pending| {
                if pending && count.get() == 16 {
                    clients.revoke(0).unwrap();
                    waits += 1;
                }
                1
            }
        ),
        Err(Error::Uncertain)
    );
    assert!(waits >= 3);
    assert_eq!(count.get(), 17); // Header submission must settle; never roll back.
}
