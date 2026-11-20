// SPDX-License-Identifier: Apache-2.0
use super::{content, control, open, record, request};
use crate::volume_disk::Disk;
use core::cell::Cell;
use rustic_file_service::Server7;
use rustic_fs::{PreventionReason, format7::RecordState};
use rustic_sdk::abi::files::{admission::State, operation, reference::Version};

#[inline(never)]
pub(super) fn verify(disk: &mut Disk<'_>, phase: u64) {
    let volume = open(disk, phase);
    let mut requests = [request(volume, 51), request(volume, 52)];
    if phase == 2 {
        for request in &mut requests {
            request.expected_version = Version::new(record(volume, *request).previous).unwrap();
        }
    }
    let mut server = Server7::new(volume);
    if phase == 1 {
        control::seed(&mut server, disk);
    }
    check(&mut server, disk, requests);
}

fn same_status(
    server: &mut Server7<'_>,
    disk: &mut Disk<'_>,
    caller: rustic_file_service::Grant7,
    request: operation::Replacement,
) -> rustic_sdk::abi::files::admission::Status {
    let record = record(server.volume(), request);
    let id = rustic_sdk::abi::files::admission::AdmissionId::new(
        server.volume().header().unwrap().lineage,
        record.admission_number,
    )
    .unwrap();
    control::status(server, disk, caller, id)
}

#[inline(never)]
fn check(server: &mut Server7<'_>, disk: &mut Disk<'_>, requests: [operation::Replacement; 2]) {
    content(server.volume(), disk, b"after");
    let start = disk.writes;
    for (request, state) in [
        (requests[0], State::Cancelled),
        (requests[1], State::Committed),
    ] {
        let retained = record(server.volume(), request);
        assert_eq!(
            retained.state,
            if state == State::Cancelled {
                RecordState::Cancelled
            } else {
                RecordState::AdmittedCommitted
            }
        );
        assert_eq!(
            retained.prevention,
            (state == State::Cancelled).then_some(PreventionReason::AuthorityLost)
        );
        let reader = control::grant(server, false);
        let expected = same_status(server, disk, reader, request);
        assert_eq!(expected.state, state);
        let foreign = control::foreign(server, false);
        assert_eq!(
            control::try_status(server, disk, foreign, expected.id),
            Err(rustic_sdk::abi::files::Error::OutcomeUnknown)
        );
        let caller = control::grant(server, true);
        assert_eq!(
            control::cancel(server, disk, caller, expected.id).state,
            state
        );
        let bytes = if state == State::Cancelled {
            b"cancelled bytes".as_slice()
        } else {
            b"after"
        };
        let count = Cell::new(0);
        assert_eq!(
            control::admit_with_control(server, disk, caller, request, bytes, &count, |_| 1),
            Ok(expected)
        );
        assert_eq!(
            control::admit_with_control(server, disk, caller, request, b"conflict", &count, |_| 1,),
            Err(rustic_sdk::abi::files::Error::IdempotencyConflict)
        );
        assert_eq!(
            control::execute_with_control(server, disk, caller, expected.id, &count, |_| 1),
            Ok(expected)
        );
    }
    assert_eq!(disk.writes, start);
}
