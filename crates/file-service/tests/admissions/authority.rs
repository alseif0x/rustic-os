// SPDX-License-Identifier: Apache-2.0
use super::super::*;

#[test]
fn restart_and_identical_retry_only_inspect_until_fresh_write_authorization() {
    let (mut s, d, caller, request) = base();
    let (mut d, accepted) = accept(&mut s, d, caller, request);
    let mut s = Server::new(Volume::mount(&mut d).unwrap());
    assert_eq!(
        s.admission_status(caller, accepted.id, 1),
        Err(Error::Revoked)
    );
    let fresh = grant(&mut s, 0, 9, 4, INSPECT_RIGHT);
    let mut d = Deferred::new(d);
    d.signals.release.set(true);
    assert_eq!(
        s.admit_with(&mut d, fresh, request, b"after", |_, _| 1),
        Ok(accepted)
    );
    assert_eq!(
        s.execute_admission_with(&mut d, fresh, accepted.id, |_, _| 1),
        Err(Error::Denied)
    );
    assert_eq!(d.signals.submitted.get(), 0);
    let fresh = grant(&mut s, 0, 9, 4, INSPECT_RIGHT | WRITE_RIGHT);
    let committed = s
        .execute_admission_with(&mut d, fresh, accepted.id, |_, _| 1)
        .unwrap();
    assert_eq!(committed.state, State::Committed);
    assert!(committed.terminal > accepted.id.number);
    assert_eq!(d.signals.submitted.get(), 17);
    let fresh = grant(&mut s, 0, 9, 4, INSPECT_RIGHT);
    assert_eq!(
        s.execute_admission_with(&mut d, fresh, accepted.id, |_, _| 1),
        Ok(committed)
    );
    assert_eq!(d.signals.submitted.get(), 17);
}

#[test]
fn retained_identity_hides_other_subjects_scopes_and_rejects_foreign_peers_without_io() {
    let (mut s, d, caller, request) = base();
    let (d, accepted) = accept(&mut s, d, caller, request);
    let mut d = Deferred::new(d);
    let other = s.volume.lookup(4, b"b").unwrap().id;
    for (subject, scope) in [(10, 4), (9, other)] {
        let foreign = grant(&mut s, 1, subject, scope, INSPECT_RIGHT | WRITE_RIGHT);
        assert_eq!(
            s.admission_status(foreign, accepted.id, 1),
            Err(Error::OutcomeUnknown)
        );
        assert_eq!(
            s.execute_admission_with(&mut d, foreign, accepted.id, |_, _| 1),
            Err(Error::OutcomeUnknown)
        );
    }
    let foreign = Caller { peer: 99, ..caller };
    assert_eq!(
        s.admit_with(&mut d, foreign, request, b"after", |_, _| 1),
        Err(Error::Denied)
    );
    assert_eq!(d.signals.submitted.get(), 0);
}

#[test]
fn expired_or_detached_helper_retires_pending_effect_but_an_unrelated_root_does_not() {
    for mode in 0..4 {
        let (mut s, d, caller, request) = base();
        let (d, accepted) = accept(&mut s, d, caller, request);
        let root = grant(&mut s, 1, 9, 4, INSPECT_RIGHT | WRITE_RIGHT);
        let caller = if mode == 0 {
            let g = s.grant_at(0).unwrap();
            Caller {
                context: s.derive(0, root.context, g, 1).unwrap(),
                ..caller
            }
        } else if mode == 1 {
            let mut g = s.grant_at(0).unwrap();
            g.expires = 5;
            Caller {
                context: s.grant(0, g).unwrap(),
                ..caller
            }
        } else {
            caller
        };
        let mut d = Deferred::new(d);
        let signals = d.signals.clone();
        let result = s.execute_admission_with(&mut d, caller, accepted.id, |clients, pending| {
            if pending {
                if mode == 2 {
                    clients.detach(0);
                } else {
                    clients.revoke_root(root.context);
                }
                signals.release.set(true);
            }
            if mode == 1 && signals.submitted.get() > 0 {
                5
            } else {
                1
            }
        });
        if mode == 3 {
            assert_eq!(result.unwrap().state, State::Committed);
        } else {
            assert_eq!(
                result,
                Err(if mode == 1 {
                    Error::Expired
                } else {
                    Error::Revoked
                })
            );
        }
        assert_eq!(
            s.volume
                .admission_by_id(9, accepted.id)
                .unwrap()
                .status
                .state,
            if mode == 3 {
                State::Committed
            } else {
                State::Cancelled
            }
        );
    }
}

#[test]
fn revocation_during_historical_retry_does_not_cancel_someone_elses_retained_work() {
    let (mut s, d, caller, request) = base();
    let (d, accepted) = accept(&mut s, d, caller, request);
    let mut d = Deferred::new(d);
    let mut calls = 0;
    assert_eq!(
        s.admit_with(&mut d, caller, request, b"after", |clients, _| {
            calls += 1;
            if calls == 2 {
                clients.revoke(0).unwrap();
            }
            1
        }),
        Err(Error::Revoked)
    );
    assert_eq!(
        s.volume.admission_by_id(9, accepted.id).unwrap().status,
        accepted
    );
    assert_eq!(d.signals.submitted.get(), 0);
}
