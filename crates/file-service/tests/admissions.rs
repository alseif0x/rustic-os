// SPDX-License-Identifier: Apache-2.0
mod support;
mod admissions {
    mod activity;
    mod authority;
    mod control;
    mod scheduling;
    mod wire;
}
use rustic_abi::files::{Error, INSPECT_RIGHT, WRITE_RIGHT};
use rustic_file_service::{Caller, Grant, Server};
use rustic_fs::{AdmissionState as State, Replacement, Retry, Volume};
use support::{Memory, deferred::Deferred};

fn base() -> (Server, Memory, Caller, Replacement) {
    let (mut s, mut d, id, _) = support::setup();
    s.volume.enable_recovery(&mut d, [7; 16]).unwrap();
    s.volume.enable_operations(&mut d).unwrap();
    s.volume.enable_admissions(&mut d).unwrap();
    let request = Replacement {
        workspace: 4,
        id,
        version: s.volume.stat(id).unwrap().version,
        retry: Retry {
            lineage: [7; 16],
            epoch: 1,
            key: 42,
        },
    };
    let caller = grant(&mut s, 0, 9, 4, INSPECT_RIGHT | WRITE_RIGHT);
    (s, d, caller, request)
}
fn grant(s: &mut Server, slot: usize, subject: u64, scope: u32, rights: u8) -> Caller {
    let peer = 10 + slot as u64;
    let context = s
        .grant(
            slot,
            Grant {
                peer,
                endpoint: 1 + slot as u64,
                scope,
                rights,
                generation: 0,
                expires: 0,
                subject,
            },
        )
        .unwrap();
    Caller {
        slot,
        peer,
        context,
    }
}
fn accept(
    s: &mut Server,
    d: Memory,
    caller: Caller,
    request: Replacement,
) -> (Memory, rustic_fs::AdmissionStatus) {
    let mut d = Deferred::new(d);
    d.signals.release.set(true);
    let status = s
        .admit_with(&mut d, caller, request, b"after", |_, _| 1)
        .unwrap();
    assert_eq!(status.state, State::Admitted);
    (d.disk, status)
}
