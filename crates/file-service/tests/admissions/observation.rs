// SPDX-License-Identifier: Apache-2.0
use crate::*;
use rustic_abi::files::{CANCEL_RIGHT, Packet, READ_RIGHT, admission as a};
use rustic_file_service::ExecutionQueue;

fn packet(old: rustic_fs::AdmissionStatus, caller: Caller, op: u8) -> Packet {
    a::AdmissionId::new(old.id.lineage, old.id.number)
        .unwrap()
        .packet(op, caller.context)
        .unwrap()
}
fn observe(
    s: &Server,
    queue: &mut ExecutionQueue,
    caller: Caller,
    old: rustic_fs::AdmissionStatus,
) -> a::Observation {
    let coarse = a::Observation::decode(&s.scheduling_request(
        queue,
        caller,
        packet(old, caller, a::OBSERVE),
        1,
    ))
    .unwrap();
    let mut p = packet(old, caller, a::OBSERVE);
    p.arg = a::OBSERVATION_V2;
    let detailed = a::ObservationV2::decode(&s.scheduling_request(queue, caller, p, 1)).unwrap();
    assert_eq!(detailed.coarse(), coarse);
    if let a::ObservationV2::Retained { status, prevention } = detailed {
        assert_eq!(prevention.is_some(), status.state == a::State::Cancelled);
    }
    coarse
}

#[test]
fn one_identity_spans_preparation_live_settlement_completion_and_restart() {
    let (mut s, d, owner, request) = base();
    let (d, old) = accept(&mut s, d, owner, request);
    let mut queue = ExecutionQueue::new();
    let prepared = observe(&s, &mut queue, owner, old);
    assert!(matches!(
        prepared,
        a::Observation::Retained(a::Status {
            state: a::State::Admitted,
            ..
        })
    ));
    assert!(queue.is_empty());
    let sequence = s.volume.sequence();
    a::Activity::decode(&s.scheduling_request(
        &mut queue,
        owner,
        packet(old, owner, a::SCHEDULE),
        1,
    ))
    .unwrap();
    assert!(matches!(
        observe(&s, &mut queue, owner, old),
        a::Observation::Active(a::Activity {
            phase: a::ActivityPhase::Queued,
            ..
        })
    ));
    assert_eq!(s.volume.sequence(), sequence);
    let mut d = Deferred::new(d);
    d.signals.release.set(true);
    let mut phases = Vec::new();
    s.run_scheduled(&mut d, &mut queue, 1, |clients, active, queue| {
        if active.pending() {
            let view = a::Observation::decode(&queue.request(
                clients,
                Some(active),
                owner,
                packet(old, owner, a::OBSERVE),
                1,
            ))
            .unwrap();
            let mut p = packet(old, owner, a::OBSERVE);
            p.arg = a::OBSERVATION_V2;
            let detailed =
                a::ObservationV2::decode(&queue.request(clients, Some(active), owner, p, 1))
                    .unwrap();
            assert_eq!(detailed.coarse(), view);
            let explicit = a::ObservationV2::decode(&active.request(clients, owner, p, 1)).unwrap();
            assert_eq!(explicit, detailed);
            assert_eq!(view.id(), prepared.id());
            let a::Observation::Active(v) = view else {
                panic!("unsettled publication claimed a retained outcome")
            };
            assert!(v.io_pending);
            phases.push(v.phase);
        }
        1
    })
    .unwrap()
    .unwrap();
    assert!(phases.contains(&a::ActivityPhase::Running));
    assert!(phases.contains(&a::ActivityPhase::Settling));
    let completed = observe(&s, &mut queue, owner, old);
    let a::Observation::Retained(v) = completed else {
        panic!()
    };
    assert_eq!(v.state, a::State::Committed);
    assert_eq!(v.id, prepared.id());
    assert_eq!(
        v.completion().unwrap().sequence(),
        s.volume.stat(request.id).unwrap().version
    );
    let mut restarted = Server::new(Volume::mount(&mut d.disk).unwrap());
    let fresh = grant(&mut restarted, 0, 9, 4, INSPECT_RIGHT);
    let before = restarted.volume.sequence();
    assert_eq!(
        observe(&restarted, &mut ExecutionQueue::new(), fresh, old),
        completed
    );
    assert_eq!(restarted.volume.sequence(), before);
}

#[test]
fn observed_scheduling_is_not_recovered_as_automatic_work_and_cancelled_has_no_live_flags() {
    let (mut s, d, owner, request) = base();
    let (mut d, old) = accept(&mut s, d, owner, request);
    let mut queue = ExecutionQueue::new();
    let prepared = observe(&s, &mut queue, owner, old);
    a::Activity::decode(&s.scheduling_request(
        &mut queue,
        owner,
        packet(old, owner, a::SCHEDULE),
        1,
    ))
    .unwrap();
    let mut restarted = Server::new(Volume::mount(&mut d).unwrap());
    let fresh = grant(&mut restarted, 0, 9, 4, INSPECT_RIGHT | CANCEL_RIGHT);
    let mut fresh_queue = ExecutionQueue::new();
    assert_eq!(observe(&restarted, &mut fresh_queue, fresh, old), prepared);
    assert!(fresh_queue.is_empty());
    restarted
        .volume
        .cancel_admission(&mut d, 9, old.id)
        .unwrap();
    let a::Observation::Retained(v) = observe(&restarted, &mut fresh_queue, fresh, old) else {
        panic!()
    };
    assert_eq!(v.state, a::State::Cancelled);
    assert!(v.completion().is_none());
}

#[test]
fn observation_rechecks_authority_hides_unknown_records_and_rejects_newer_profiles() {
    for profile in [a::OBSERVATION_VERSION, a::OBSERVATION_V2] {
        for (rights, subject, scope, expected) in [
            (READ_RIGHT, 9, 4, Error::Denied),
            (WRITE_RIGHT, 9, 4, Error::Denied),
            (CANCEL_RIGHT, 9, 4, Error::Denied),
            (INSPECT_RIGHT, 8, 4, Error::OutcomeUnknown),
            (INSPECT_RIGHT, 9, 3, Error::OutcomeUnknown),
        ] {
            let (mut s, d, owner, request) = base();
            let (d, old) = accept(&mut s, d, owner, request);
            let peer = grant(&mut s, 1, subject, scope, rights);
            let mut queue = ExecutionQueue::new();
            let mut p = packet(old, peer, a::OBSERVE);
            p.arg = profile;
            let r = s.scheduling_request(&mut queue, peer, p, 1);
            assert_eq!(r.status, expected as u8);
            assert_eq!((r.count, r.data, r.version), (0, [0; 40], 0));
            a::Activity::decode(&s.scheduling_request(
                &mut queue,
                owner,
                packet(old, owner, a::SCHEDULE),
                1,
            ))
            .unwrap();
            let mut d = Deferred::new(d);
            d.signals.release.set(true);
            let mut denied = false;
            s.run_scheduled(&mut d, &mut queue, 1, |clients, active, queue| {
                if active.pending() && !denied {
                    denied = true;
                    let r = queue.request(clients, Some(active), peer, p, 1);
                    assert_eq!(r.status, expected as u8);
                    assert_eq!((r.count, r.data, r.version), (0, [0; 40], 0));
                }
                1
            })
            .unwrap()
            .unwrap();
            assert!(denied);
            let mut wrong = packet(old, owner, a::OBSERVE);
            wrong.version += 100;
            assert_eq!(
                s.scheduling_request(&mut queue, owner, wrong, 1).status,
                Error::OutcomeUnknown as u8
            );
            wrong.arg = a::OBSERVATION_V2 + 1;
            assert_eq!(
                s.scheduling_request(&mut queue, owner, wrong, 1).status,
                Error::UnsupportedVersion as u8
            );
            s.revoke(0).unwrap();
            assert_eq!(
                s.scheduling_request(&mut queue, owner, packet(old, owner, a::OBSERVE), 1)
                    .status,
                Error::Revoked as u8
            );
        }
    }
}
