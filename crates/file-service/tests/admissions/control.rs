// SPDX-License-Identifier: Apache-2.0
use super::super::*;

#[test]
fn admission_revocation_at_every_pending_command_never_executes_and_retires_late_acceptance() {
    for cut in 1..=14 {
        let (mut s, d, caller, request) = base();
        let mut d = Deferred::new(d);
        let signals = d.signals.clone();
        let mut waits = 0;
        let result = s.admit_with(&mut d, caller, request, b"after", |clients, pending| {
            if pending && signals.submitted.get() == cut {
                waits += 1;
                clients.revoke(caller.slot).unwrap();
                signals.release.set(waits >= 4);
            } else {
                signals.release.set(true);
            }
            1
        });
        assert_eq!(result, Err(Error::Revoked));
        assert!(waits >= 4);
        let late = cut >= 13;
        assert_eq!(signals.submitted.get(), if late { 28 } else { cut });
        assert_eq!(signals.settled.get(), signals.submitted.get());
        let mounted = Volume::mount(&mut d.disk).unwrap();
        assert_eq!(mounted.stat(request.id).unwrap().version, request.version);
        let old = mounted.admission_by_retry(9, 4, request.retry);
        if late {
            assert_eq!(old.unwrap().status.state, State::Cancelled);
        } else {
            assert!(matches!(old, Err(rustic_fs::Error::OutcomeUnknown)));
        }
    }
}

#[test]
fn execution_revocation_at_every_pending_command_retires_only_prevented_effects() {
    for cut in 1..=17 {
        let (mut s, d, caller, request) = base();
        let (d, accepted) = accept(&mut s, d, caller, request);
        let mut d = Deferred::new(d);
        let signals = d.signals.clone();
        let mut waits = 0;
        let result = s.execute_admission_with(&mut d, caller, accepted.id, |clients, pending| {
            if pending && signals.submitted.get() == cut {
                waits += 1;
                clients.revoke(caller.slot).unwrap();
                signals.release.set(waits >= 4);
            } else {
                signals.release.set(true);
            }
            1
        });
        assert!(waits >= 4);
        let late = cut >= 16;
        assert_eq!(
            result,
            Err(if late {
                Error::Uncertain
            } else {
                Error::Revoked
            })
        );
        assert_eq!(signals.submitted.get(), if late { 17 } else { cut + 14 });
        assert_eq!(signals.settled.get(), signals.submitted.get());
        let mounted = Volume::mount(&mut d.disk).unwrap();
        let old = mounted.admission_by_id(9, accepted.id).unwrap();
        assert_eq!(
            old.status.state,
            if late {
                State::Committed
            } else {
                State::Cancelled
            }
        );
        assert_eq!(
            mounted.stat(request.id).unwrap().length,
            if late { 5 } else { 0 }
        );
        assert_eq!(old.receipt.is_some(), late);
    }
}

#[test]
fn errors_at_every_terminal_command_remain_uncertain_and_require_recovery() {
    // One drained data command followed by all 14 cancellation commands.
    for fail in 2..=15 {
        let (mut s, d, caller, request) = base();
        let (d, accepted) = accept(&mut s, d, caller, request);
        let mut d = Deferred::new(d);
        let signals = d.signals.clone();
        let result = s.execute_admission_with(&mut d, caller, accepted.id, |clients, pending| {
            if pending {
                clients.detach(caller.slot);
            }
            signals.release.set(true);
            signals.fail.set(signals.submitted.get() == fail);
            1
        });
        assert_eq!(result, Err(Error::Uncertain));
        assert_eq!(signals.submitted.get(), fail);
        assert!(matches!(
            s.volume.stat(request.id),
            Err(rustic_fs::Error::Uncertain)
        ));
        let fresh = grant(&mut s, 0, 9, 0, INSPECT_RIGHT);
        assert_eq!(
            s.admission_status(fresh, accepted.id, 1),
            Err(Error::Uncertain)
        );
        let mounted = Volume::mount(&mut d.disk).unwrap();
        let old = mounted.admission_by_id(9, accepted.id).unwrap();
        assert_eq!(
            old.status.state,
            if fail >= 14 {
                State::Cancelled
            } else {
                State::Admitted
            }
        );
        assert_eq!(mounted.stat(request.id).unwrap().version, request.version);
    }
}

#[test]
fn terminal_housekeeping_keeps_owner_control_live_after_original_client_disappears() {
    let (mut s, d, caller, request) = base();
    let (d, accepted) = accept(&mut s, d, caller, request);
    let mut d = Deferred::new(d);
    let signals = d.signals.clone();
    let mut holds = [0; 16];
    let result = s.execute_admission_with(&mut d, caller, accepted.id, |clients, pending| {
        if pending {
            clients.detach(caller.slot);
            let command = signals.submitted.get();
            holds[command] += 1;
            signals.release.set(holds[command] >= 3);
        }
        1
    });
    assert_eq!(result, Err(Error::Revoked));
    assert!(holds[1..].iter().all(|n| *n == 3));
    assert_eq!(
        s.volume
            .admission_by_id(9, accepted.id)
            .unwrap()
            .status
            .state,
        State::Cancelled
    );
}
