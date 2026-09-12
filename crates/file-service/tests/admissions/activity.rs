// SPDX-License-Identifier: Apache-2.0
use crate::*;
use rustic_abi::files::{CANCEL_RIGHT, READ_RIGHT, admission as a};

fn id(status: rustic_fs::AdmissionStatus) -> a::AdmissionId {
    a::AdmissionId::new(status.id.lineage, status.id.number).unwrap()
}

#[test]
fn public_stop_at_every_pending_command_reports_no_speculative_terminal_success() {
    for cut in 0..17 {
        let (mut s, d, executor, request) = base();
        let cancel = grant(&mut s, 1, 9, request.id, CANCEL_RIGHT);
        let (d, admitted) = accept(&mut s, d, executor, request);
        let mut d = Deferred::new(d);
        let signals = d.signals.clone();
        signals.release.set(true);
        let mut requested = false;
        let result = s
            .execute_admission_active_with(&mut d, executor, admitted.id, 1, |clients, active| {
                if !requested && signals.submitted.get() == cut + 1 && active.pending() {
                    signals.release.set(false);
                    let p = id(admitted).packet(a::ACTIVITY, executor.context).unwrap();
                    let before =
                        a::Activity::decode(&active.request(clients, executor, p, 1)).unwrap();
                    assert!(before.io_pending);
                    assert!(!before.cancel_requested);
                    let p = id(admitted)
                        .packet(a::REQUEST_CANCEL, cancel.context)
                        .unwrap();
                    let reply = active.request(clients, cancel, p, 1);
                    let ack = a::Activity::decode(&reply).unwrap();
                    assert!(ack.cancel_requested);
                    assert!(
                        a::Status::decode(&reply).is_err(),
                        "a volatile ACK is not a durable result"
                    );
                    assert_eq!(
                        ack.phase,
                        if cut < 15 {
                            a::ActivityPhase::Stopping
                        } else {
                            a::ActivityPhase::Settling
                        }
                    );
                    // Repeating is bounded and cannot submit a second disk request.
                    active.request(clients, cancel, p, 1);
                    assert_eq!(signals.submitted.get(), cut + 1);
                    requested = true;
                }
                signals.release.set(true);
                1
            })
            .unwrap();
        assert!(requested);
        assert_eq!(
            result.state,
            if cut < 15 {
                State::Cancelled
            } else {
                State::Committed
            }
        );
        let mounted = Volume::mount(&mut d.disk).unwrap();
        assert_eq!(
            mounted.admission_by_id(9, admitted.id).unwrap().status,
            result
        );
        assert_eq!(
            mounted.stat(request.id).unwrap().version == request.version,
            cut < 15
        );
    }
}

#[test]
fn active_authority_rejects_wrong_subject_scope_right_peer_and_generation() {
    for (subject, scope, rights, expected) in [
        (8, 4, CANCEL_RIGHT, Error::OutcomeUnknown),
        (9, 3, CANCEL_RIGHT, Error::OutcomeUnknown),
        (9, 4, INSPECT_RIGHT, Error::Denied),
        (9, 4, READ_RIGHT | WRITE_RIGHT, Error::Denied),
    ] {
        let (mut s, d, executor, request) = base();
        let caller = grant(&mut s, 1, subject, scope, rights);
        let (d, admitted) = accept(&mut s, d, executor, request);
        let mut d = Deferred::new(d);
        d.signals.release.set(true);
        let mut checked = false;
        let result = s
            .execute_admission_active_with(&mut d, executor, admitted.id, 1, |clients, active| {
                if !checked {
                    let p = id(admitted)
                        .packet(a::REQUEST_CANCEL, caller.context)
                        .unwrap();
                    assert_eq!(active.request(clients, caller, p, 1).status, expected as u8);
                    let wrong = Caller {
                        peer: 999,
                        ..caller
                    };
                    assert_eq!(
                        active.request(clients, wrong, p, 1).status,
                        Error::Denied as u8
                    );
                    let stale = Caller {
                        context: caller.context + 1,
                        ..caller
                    };
                    assert_eq!(
                        active
                            .request(
                                clients,
                                stale,
                                id(admitted)
                                    .packet(a::REQUEST_CANCEL, stale.context)
                                    .unwrap(),
                                1
                            )
                            .status,
                        Error::Revoked as u8
                    );
                    checked = true;
                }
                1
            })
            .unwrap();
        assert_eq!(result.state, State::Committed);
    }
}

#[test]
fn accepted_stop_survives_canceller_revocation_but_new_requests_are_denied() {
    let (mut s, d, executor, request) = base();
    let cancel = grant(&mut s, 1, 9, request.id, CANCEL_RIGHT);
    let (d, admitted) = accept(&mut s, d, executor, request);
    let mut d = Deferred::new(d);
    d.signals.release.set(true);
    let mut checked = false;
    let result = s
        .execute_admission_active_with(&mut d, executor, admitted.id, 1, |clients, active| {
            if !checked {
                let query = id(admitted).packet(a::ACTIVITY, cancel.context).unwrap();
                assert_eq!(
                    active.request(clients, cancel, query, 1).status,
                    Error::Denied as u8
                );
                let mut p = id(admitted)
                    .packet(a::REQUEST_CANCEL, cancel.context)
                    .unwrap();
                p.version += 1;
                assert_eq!(
                    active.request(clients, cancel, p, 1).status,
                    Error::OutcomeUnknown as u8
                );
                p.version -= 1;
                assert_eq!(active.request(clients, cancel, p, 1).status, 0);
                clients.revoke(cancel.slot).unwrap();
                assert_eq!(
                    active.request(clients, cancel, p, 1).status,
                    Error::Revoked as u8
                );
                checked = true;
            }
            1
        })
        .unwrap();
    assert_eq!(result.state, State::Cancelled);
}

#[test]
fn stop_with_failed_drain_is_uncertain_and_restart_never_executes() {
    let (mut s, d, executor, request) = base();
    let cancel = grant(&mut s, 1, 9, request.id, CANCEL_RIGHT);
    let (d, admitted) = accept(&mut s, d, executor, request);
    let mut d = Deferred::new(d);
    let signals = d.signals.clone();
    let result =
        s.execute_admission_active_with(&mut d, executor, admitted.id, 1, |clients, active| {
            if active.pending() {
                let p = id(admitted)
                    .packet(a::REQUEST_CANCEL, cancel.context)
                    .unwrap();
                assert_eq!(active.request(clients, cancel, p, 1).status, 0);
                signals.release.set(true);
                signals.fail.set(true);
            }
            1
        });
    assert_eq!(result, Err(Error::Uncertain));
    assert!(s.volume.stat(request.id).is_err());
    let mounted = Volume::mount(&mut d.disk).unwrap();
    assert_eq!(
        mounted
            .admission_by_id(9, admitted.id)
            .unwrap()
            .status
            .state,
        State::Admitted
    );
    assert_eq!(mounted.stat(request.id).unwrap().version, request.version);
}

#[test]
fn cached_scope_never_extends_an_expired_cancellation_grant() {
    let (mut s, d, executor, request) = base();
    let cancel = grant(&mut s, 1, 9, request.id, CANCEL_RIGHT);
    let mut limited = s.grant_at(cancel.slot).unwrap();
    s.detach(cancel.slot);
    limited.expires = 2;
    let cancel = Caller {
        context: s.grant(cancel.slot, limited).unwrap(),
        ..cancel
    };
    let (d, admitted) = accept(&mut s, d, executor, request);
    let mut d = Deferred::new(d);
    d.signals.release.set(true);
    let result = s
        .execute_admission_active_with(&mut d, executor, admitted.id, 1, |clients, active| {
            let p = id(admitted)
                .packet(a::REQUEST_CANCEL, cancel.context)
                .unwrap();
            assert_eq!(
                active.request(clients, cancel, p, 2).status,
                Error::Expired as u8
            );
            2
        })
        .unwrap();
    assert_eq!(result.state, State::Committed);
}
