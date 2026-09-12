// SPDX-License-Identifier: Apache-2.0
use crate::*;
use rustic_abi::files::{CANCEL_RIGHT, admission as a};
use rustic_file_service::ExecutionQueue;
use rustic_fs::PreventionReason as Reason;

fn id(status: rustic_fs::AdmissionStatus) -> a::AdmissionId {
    a::AdmissionId::new(status.id.lineage, status.id.number).unwrap()
}

#[test]
fn queued_prevention_distinguishes_stop_conflict_and_lost_authority_after_remount() {
    for format5 in [false, true] {
        for cause in 0..5 {
            let (mut s, mut d, caller, request) = base();
            if format5 {
                s.volume.enable_prevention_reasons(&mut d).unwrap();
            }
            let cancel = grant(&mut s, 1, 9, request.id, CANCEL_RIGHT);
            let (mut d, admitted) = accept(&mut s, d, caller, request);
            let mut queue = ExecutionQueue::new();
            let p = id(admitted).packet(a::SCHEDULE, caller.context).unwrap();
            assert_eq!(s.scheduling_request(&mut queue, caller, p, 1).status, 0);
            let reason = match cause {
                0 => {
                    let p = id(admitted)
                        .packet(a::REQUEST_CANCEL, cancel.context)
                        .unwrap();
                    assert_eq!(s.scheduling_request(&mut queue, cancel, p, 1).status, 0);
                    // An accepted stop is not rewritten as guard loss at dispatch.
                    s.revoke(caller.slot).unwrap();
                    Reason::Requested
                }
                1 => {
                    s.volume
                        .replace(&mut d, request.id, request.version, b"human edit")
                        .unwrap();
                    Reason::VersionConflict
                }
                2 => {
                    s.revoke(caller.slot).unwrap();
                    Reason::AuthorityLost
                }
                3 => {
                    s.detach(caller.slot);
                    Reason::AuthorityLost
                }
                _ => {
                    grant(&mut s, caller.slot, 9, 4, INSPECT_RIGHT | WRITE_RIGHT);
                    Reason::AuthorityLost
                }
            };
            let version = s.volume.stat(request.id).unwrap().version;
            let mut d = Deferred::new(d);
            d.signals.release.set(true);
            let status = s
                .run_scheduled(&mut d, &mut queue, 1, |_, _, _| 1)
                .unwrap()
                .unwrap();
            assert_eq!(status.state, State::Cancelled);
            assert_eq!(
                status.prevention,
                Some(if format5 { reason } else { Reason::Unknown })
            );
            let mounted = Volume::mount(&mut d.disk).unwrap();
            assert_eq!(
                mounted.admission_by_id(9, admitted.id).unwrap().status,
                status
            );
            assert_eq!(mounted.stat(request.id).unwrap().version, version);
            assert!(queue.is_empty());
            let mut restarted = Server::new(mounted);
            let inspector = grant(&mut restarted, 0, 9, 4, INSPECT_RIGHT);
            let sequence = restarted.volume.sequence();
            let p = a::ObservationV2::request(id(admitted), inspector.context).unwrap();
            let observed = a::ObservationV2::decode(
                &restarted.scheduling_request(&mut queue, inspector, p, 1),
            )
            .unwrap();
            let expected = if !format5 {
                a::PreventionReason::Unknown
            } else {
                match reason {
                    Reason::Requested => a::PreventionReason::Requested,
                    Reason::VersionConflict => a::PreventionReason::VersionConflict,
                    Reason::AuthorityLost => a::PreventionReason::AuthorityLost,
                    Reason::Unknown => unreachable!(),
                }
            };
            assert!(
                matches!(observed, a::ObservationV2::Retained {prevention:Some(r),..} if r==expected)
            );
            let legacy = id(admitted).packet(a::OBSERVE, inspector.context).unwrap();
            assert_eq!(
                a::Observation::decode(
                    &restarted.scheduling_request(&mut queue, inspector, legacy, 1)
                )
                .unwrap(),
                observed.coarse()
            );
            assert_eq!(restarted.volume.sequence(), sequence);
        }
    }
}

#[test]
fn live_stop_guard_loss_and_late_completion_retain_only_proven_causes() {
    for cut in 0..17 {
        for revoke in [false, true] {
            let (mut s, mut d, caller, request) = base();
            s.volume.enable_prevention_reasons(&mut d).unwrap();
            let cancel = grant(&mut s, 1, 9, request.id, CANCEL_RIGHT);
            let (d, admitted) = accept(&mut s, d, caller, request);
            let mut d = Deferred::new(d);
            let signals = d.signals.clone();
            signals.release.set(true);
            let mut stopped = false;
            let result = s.execute_admission_active_with(
                &mut d,
                caller,
                admitted.id,
                1,
                |clients, active| {
                    if !stopped && signals.submitted.get() == cut + 1 && active.pending() {
                        let p = id(admitted)
                            .packet(a::REQUEST_CANCEL, cancel.context)
                            .unwrap();
                        assert_eq!(active.request(clients, cancel, p, 1).status, 0);
                        if revoke {
                            clients.revoke(caller.slot).unwrap();
                        }
                        stopped = true;
                    }
                    1
                },
            );
            assert!(stopped);
            if revoke {
                assert_eq!(
                    result,
                    Err(if cut < 15 {
                        Error::Revoked
                    } else {
                        Error::Uncertain
                    })
                );
            } else {
                assert!(result.is_ok());
            }
            let mounted = Volume::mount(&mut d.disk).unwrap();
            let status = mounted.admission_by_id(9, admitted.id).unwrap().status;
            assert_eq!(
                status.state,
                if cut < 15 {
                    State::Cancelled
                } else {
                    State::Committed
                }
            );
            assert_eq!(
                status.prevention,
                if cut < 15 {
                    Some(if revoke {
                        Reason::AuthorityLost
                    } else {
                        Reason::Requested
                    })
                } else {
                    None
                }
            );
            assert_eq!(
                mounted.stat(request.id).unwrap().version == request.version,
                cut < 15
            );
        }
    }
}

#[test]
fn failed_drain_does_not_invent_a_persisted_stop_reason() {
    let (mut s, mut d, caller, request) = base();
    s.volume.enable_prevention_reasons(&mut d).unwrap();
    let cancel = grant(&mut s, 1, 9, request.id, CANCEL_RIGHT);
    let (d, admitted) = accept(&mut s, d, caller, request);
    let mut d = Deferred::new(d);
    let signals = d.signals.clone();
    let result =
        s.execute_admission_active_with(&mut d, caller, admitted.id, 1, |clients, active| {
            if active.pending() {
                let p = id(admitted)
                    .packet(a::REQUEST_CANCEL, cancel.context)
                    .unwrap();
                assert_eq!(active.request(clients, cancel, p, 1).status, 0);
                signals.fail.set(true);
                signals.release.set(true);
            }
            1
        });
    assert_eq!(result, Err(Error::Uncertain));
    assert!(s.volume.stat(request.id).is_err());
    let mounted = Volume::mount(&mut d.disk).unwrap();
    assert_eq!(
        mounted.admission_by_id(9, admitted.id).unwrap().status,
        admitted
    );
}
