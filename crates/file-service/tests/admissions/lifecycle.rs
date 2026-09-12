// SPDX-License-Identifier: Apache-2.0
use crate::*;
use rustic_abi::files::{CANCEL_RIGHT, Packet, admission as a, lifecycle as l};
use rustic_file_service::ExecutionQueue;

fn id(status: rustic_fs::AdmissionStatus) -> a::AdmissionId {
    a::AdmissionId::new(status.id.lineage, status.id.number).unwrap()
}
fn ack(p: Packet, expected: l::Disposition) {
    assert_eq!(l::CancelAck::decode(&p).unwrap().disposition, expected);
    assert_eq!(p.count, 16);
    assert_eq!(&p.data[16..], &[0; 24]);
}
fn empty_error(p: Packet, error: Error) {
    assert_eq!(p.status, error as u8);
    assert_eq!(
        (p.count, p.id, p.arg, p.version, p.data),
        (0, 0, 0, 0, [0; 40])
    );
}

#[test]
fn prepared_cancel_never_needs_write_or_inspect_and_restart_drops_only_volatile_work() {
    for settle in [false, true] {
        let (mut s, mut d, owner, request) = base();
        s.volume.enable_prevention_reasons(&mut d).unwrap();
        let cancel = grant(&mut s, 1, 9, request.id, CANCEL_RIGHT);
        let (mut d, admitted) = accept(&mut s, d, owner, request);
        let mut queue = ExecutionQueue::new();
        let p = l::CancelAck::request(id(admitted), cancel.context).unwrap();
        let before = s.volume.sequence();
        ack(
            s.scheduling_request(&mut queue, cancel, p, 1),
            l::Disposition::Requested,
        );
        ack(
            s.scheduling_request(&mut queue, cancel, p, 1),
            l::Disposition::AlreadyRequested,
        );
        assert_eq!(s.volume.sequence(), before);
        let inspect = a::ObservationV2::request(id(admitted), cancel.context).unwrap();
        empty_error(
            s.scheduling_request(&mut queue, cancel, inspect, 1),
            Error::Denied,
        );
        s.revoke(cancel.slot).unwrap();
        empty_error(
            s.scheduling_request(&mut queue, cancel, p, 1),
            Error::Revoked,
        );
        if settle {
            let mut disk = Deferred::new(d);
            disk.signals.release.set(true);
            let result = s
                .run_scheduled(&mut disk, &mut queue, 1, |_, _, _| 1)
                .unwrap()
                .unwrap();
            assert_eq!(
                result.prevention,
                Some(rustic_fs::PreventionReason::Requested)
            );
            d = disk.disk;
        }
        let mut fresh = Server::new(Volume::mount(&mut d).unwrap());
        let caller = grant(&mut fresh, 0, 9, 4, INSPECT_RIGHT | CANCEL_RIGHT);
        let mut queue = ExecutionQueue::new();
        let mut disk = Deferred::new(d);
        assert!(
            fresh
                .run_scheduled(&mut disk, &mut queue, 1, |_, _, _| 1)
                .is_none()
        );
        assert_eq!(
            fresh.volume.stat(request.id).unwrap().version,
            request.version
        );
        let query = a::ObservationV2::request(id(admitted), caller.context).unwrap();
        let view =
            a::ObservationV2::decode(&fresh.scheduling_request(&mut queue, caller, query, 1))
                .unwrap();
        assert_eq!(
            l::Operation::try_from(view).unwrap().state,
            if settle {
                l::State::Cancelled
            } else {
                l::State::Prepared
            }
        );
        let p = l::CancelAck::request(id(admitted), caller.context).unwrap();
        ack(
            fresh.scheduling_request(&mut queue, caller, p, 1),
            if settle {
                l::Disposition::TooLate
            } else {
                l::Disposition::Requested
            },
        );
    }
}

#[test]
fn minimal_cancel_at_every_pending_command_keeps_late_completion_honest() {
    for cut in 0..17 {
        let (mut s, mut d, executor, request) = base();
        s.volume.enable_prevention_reasons(&mut d).unwrap();
        let cancel = grant(&mut s, 1, 9, request.id, CANCEL_RIGHT);
        let (d, admitted) = accept(&mut s, d, executor, request);
        let mut disk = Deferred::new(d);
        let signals = disk.signals.clone();
        signals.release.set(true);
        let mut checked = false;
        let result = s
            .execute_admission_active_with(
                &mut disk,
                executor,
                admitted.id,
                1,
                |clients, active| {
                    if !checked && active.pending() && signals.submitted.get() == cut + 1 {
                        let p = l::CancelAck::request(id(admitted), cancel.context).unwrap();
                        ack(
                            active.request(clients, cancel, p, 1),
                            l::Disposition::Requested,
                        );
                        ack(
                            active.request(clients, cancel, p, 1),
                            l::Disposition::AlreadyRequested,
                        );
                        empty_error(
                            active.request(
                                clients,
                                cancel,
                                a::ObservationV2::request(id(admitted), cancel.context).unwrap(),
                                1,
                            ),
                            Error::Denied,
                        );
                        checked = true;
                    }
                    1
                },
            )
            .unwrap();
        assert!(checked);
        assert_eq!(
            result.state,
            if cut < 15 {
                State::Cancelled
            } else {
                State::Committed
            }
        );
        assert_eq!(
            Volume::mount(&mut disk.disk)
                .unwrap()
                .admission_by_id(9, admitted.id)
                .unwrap()
                .status,
            result
        );
        let mut queue = ExecutionQueue::new();
        ack(
            s.scheduling_request(
                &mut queue,
                cancel,
                l::CancelAck::request(id(admitted), cancel.context).unwrap(),
                1,
            ),
            l::Disposition::TooLate,
        );
    }
}

#[test]
fn cancellation_checks_right_scope_subject_and_profile_without_disclosing_results() {
    for (rights, subject, scope, expected) in [
        (INSPECT_RIGHT, 9, 4, Error::Denied),
        (WRITE_RIGHT, 9, 4, Error::Denied),
        (CANCEL_RIGHT, 8, 4, Error::OutcomeUnknown),
        (CANCEL_RIGHT, 9, 3, Error::OutcomeUnknown),
    ] {
        let (mut s, d, owner, request) = base();
        let caller = grant(&mut s, 1, subject, scope, rights);
        let (_, admitted) = accept(&mut s, d, owner, request);
        let mut queue = ExecutionQueue::new();
        let mut p = l::CancelAck::request(id(admitted), caller.context).unwrap();
        empty_error(s.scheduling_request(&mut queue, caller, p, 1), expected);
        p.arg = 1;
        empty_error(
            s.scheduling_request(&mut queue, caller, p, 1),
            Error::UnsupportedVersion,
        );
        assert!(queue.is_empty());
    }
}

#[test]
fn failed_drain_discards_active_and_prepared_stop_tickets_without_replay() {
    let (mut s, mut d, executor, request) = base();
    s.volume.enable_prevention_reasons(&mut d).unwrap();
    let cancel = grant(&mut s, 1, 9, request.id, CANCEL_RIGHT);
    let (d, first) = accept(&mut s, d, executor, request);
    let (d, second) = accept(
        &mut s,
        d,
        executor,
        Replacement {
            retry: Retry {
                key: 43,
                ..request.retry
            },
            ..request
        },
    );
    let mut queue = ExecutionQueue::new();
    assert_eq!(
        s.scheduling_request(
            &mut queue,
            executor,
            id(first).packet(a::SCHEDULE, executor.context).unwrap(),
            1
        )
        .status,
        0
    );
    let mut disk = Deferred::new(d);
    let signals = disk.signals.clone();
    let mut stopped = false;
    assert_eq!(
        s.run_scheduled(&mut disk, &mut queue, 1, |clients, active, queue| {
            if active.pending() && !stopped {
                for record in [second, first] {
                    let p = l::CancelAck::request(id(record), cancel.context).unwrap();
                    ack(
                        queue.request(clients, Some(active), cancel, p, 1),
                        l::Disposition::Requested,
                    );
                }
                signals.fail.set(true);
                signals.release.set(true);
                stopped = true;
            }
            1
        })
        .unwrap(),
        Err(Error::Uncertain)
    );
    assert!(stopped && queue.is_empty());
    let mut restarted = Server::new(Volume::mount(&mut disk.disk).unwrap());
    for record in [first, second] {
        assert_eq!(
            restarted
                .volume
                .admission_by_id(9, record.id)
                .unwrap()
                .status,
            record
        );
    }
    assert!(
        restarted
            .run_scheduled(&mut disk, &mut ExecutionQueue::new(), 1, |_, _, _| 1)
            .is_none()
    );
    assert_eq!(
        restarted.volume.stat(request.id).unwrap().version,
        request.version
    );
}
