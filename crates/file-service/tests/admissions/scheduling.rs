// SPDX-License-Identifier: Apache-2.0
use crate::*;
use rustic_abi::files::{CANCEL_RIGHT, READ_RIGHT, admission as a};
use rustic_file_service::ExecutionQueue;

fn id(status: rustic_fs::AdmissionStatus) -> a::AdmissionId {
    a::AdmissionId::new(status.id.lineage, status.id.number).unwrap()
}
fn call(
    s: &Server,
    queue: &mut ExecutionQueue,
    caller: Caller,
    status: rustic_fs::AdmissionStatus,
    op: u8,
) -> rustic_abi::files::Packet {
    s.scheduling_request(
        queue,
        caller,
        id(status).packet(op, caller.context).unwrap(),
        1,
    )
}

#[test]
fn acceptance_is_separate_from_execution_and_an_active_client_can_enqueue_and_cancel_a_peer() {
    let (mut s, d, executor, request) = base();
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
    let before = s.volume.sequence();
    let ack = a::Activity::decode(&call(&s, &mut queue, executor, first, a::SCHEDULE)).unwrap();
    assert_eq!(ack.phase, a::ActivityPhase::Queued);
    assert!(!ack.io_pending);
    assert_eq!(
        s.volume.sequence(),
        before,
        "scheduling itself does not write storage"
    );
    assert_eq!(s.volume.stat(request.id).unwrap().version, request.version);
    let mut disk = Deferred::new(d);
    let signals = disk.signals.clone();
    signals.release.set(true);
    let mut observed = false;
    assert_eq!(
        s.run_scheduled(&mut disk, &mut queue, 1, |clients, active, queue| {
            if active.pending() && !observed {
                observed = true;
                let p = id(second).packet(a::SCHEDULE, executor.context).unwrap();
                let ack =
                    a::Activity::decode(&queue.request(clients, Some(active), executor, p, 1))
                        .unwrap();
                assert_eq!(ack.phase, a::ActivityPhase::Queued);
                // Duplicate scheduling cannot allocate another ticket or change its owner.
                assert_eq!(
                    a::Activity::decode(&queue.request(clients, Some(active), executor, p, 1)),
                    Ok(ack)
                );
                let p = id(second)
                    .packet(a::REQUEST_CANCEL, cancel.context)
                    .unwrap();
                let stopped =
                    a::Activity::decode(&queue.request(clients, Some(active), cancel, p, 1))
                        .unwrap();
                assert_eq!(stopped.phase, a::ActivityPhase::Queued);
                assert!(stopped.cancel_requested && !stopped.io_pending);
                let p = id(first).packet(a::ACTIVITY, executor.context).unwrap();
                let running =
                    a::Activity::decode(&queue.request(clients, Some(active), executor, p, 1))
                        .unwrap();
                assert_eq!(running.phase, a::ActivityPhase::Running);
                assert!(running.io_pending && !running.cancel_requested);
            }
            1
        })
        .unwrap()
        .unwrap()
        .state,
        State::Committed
    );
    assert!(observed && !queue.is_empty());
    assert_eq!(
        s.run_scheduled(&mut disk, &mut queue, 1, |_, _, _| 1)
            .unwrap()
            .unwrap()
            .state,
        State::Cancelled
    );
    assert!(queue.is_empty());
    let mounted = Volume::mount(&mut disk.disk).unwrap();
    assert_eq!(
        mounted.admission_by_id(9, first.id).unwrap().status.state,
        State::Committed
    );
    assert_eq!(
        mounted.admission_by_id(9, second.id).unwrap().status.state,
        State::Cancelled
    );
    assert_eq!(
        mounted.stat(request.id).unwrap().version,
        s.volume
            .admission_by_id(9, first.id)
            .unwrap()
            .status
            .terminal
    );
}

#[test]
fn queued_work_rechecks_human_edits_and_never_overwrites_a_newer_version() {
    let (mut s, d, caller, request) = base();
    let (mut d, admitted) = accept(&mut s, d, caller, request);
    let mut queue = ExecutionQueue::new();
    a::Activity::decode(&call(&s, &mut queue, caller, admitted, a::SCHEDULE)).unwrap();
    let human = s
        .volume
        .replace(&mut d, request.id, request.version, b"human edit")
        .unwrap();
    let mut d = Deferred::new(d);
    d.signals.release.set(true);
    assert_eq!(
        s.run_scheduled(&mut d, &mut queue, 1, |_, _, _| 1)
            .unwrap()
            .unwrap()
            .state,
        State::Cancelled
    );
    assert_eq!(s.volume.stat(request.id).unwrap().version, human.version);
    let mut bytes = [0; 32];
    let len = s
        .volume
        .read(&mut d.disk, request.id, 0, &mut bytes)
        .unwrap();
    assert_eq!(&bytes[..len], b"human edit");
}

#[test]
fn queued_authority_is_never_restored_by_revoke_detach_expiry_or_regrant() {
    for change in 0..4 {
        let (mut s, d, caller, request) = base();
        if change == 2 {
            let mut g = s.grant_at(0).unwrap();
            g.expires = 3;
            s.grant(0, g).unwrap();
        }
        let caller = Caller {
            context: s.grant_at(0).unwrap().generation,
            ..caller
        };
        let (d, admitted) = accept(&mut s, d, caller, request);
        let mut queue = ExecutionQueue::new();
        a::Activity::decode(&call(&s, &mut queue, caller, admitted, a::SCHEDULE)).unwrap();
        match change {
            0 => {
                s.revoke(0).unwrap();
            }
            1 => s.detach(0),
            2 => (),
            _ => {
                grant(&mut s, 0, 9, 4, INSPECT_RIGHT | WRITE_RIGHT);
            }
        }
        let mut disk = Deferred::new(d);
        disk.signals.release.set(true);
        assert_eq!(
            s.run_scheduled(&mut disk, &mut queue, 4, |_, _, _| 4)
                .unwrap()
                .unwrap()
                .state,
            State::Cancelled
        );
        assert_eq!(s.volume.stat(request.id).unwrap().version, request.version);
    }
}

#[test]
fn schedule_requires_both_inspection_and_write_and_hides_foreign_scope() {
    for (rights, subject, scope, error) in [
        (READ_RIGHT, 9, 4, Error::Denied),
        (WRITE_RIGHT, 9, 4, Error::Denied),
        (INSPECT_RIGHT, 9, 4, Error::Denied),
        (CANCEL_RIGHT, 9, 4, Error::Denied),
        (INSPECT_RIGHT | WRITE_RIGHT, 8, 4, Error::OutcomeUnknown),
        (INSPECT_RIGHT | WRITE_RIGHT, 9, 3, Error::OutcomeUnknown),
    ] {
        let (mut s, d, owner, request) = base();
        let caller = grant(&mut s, 1, subject, scope, rights);
        let (_, admitted) = accept(&mut s, d, owner, request);
        let mut queue = ExecutionQueue::new();
        assert_eq!(
            call(&s, &mut queue, caller, admitted, a::SCHEDULE).status,
            error as u8
        );
        assert!(queue.is_empty());
    }
}

#[test]
fn restart_forgets_scheduling_and_requires_a_fresh_explicit_request() {
    let (mut s, d, caller, request) = base();
    let (mut d, admitted) = accept(&mut s, d, caller, request);
    let mut queue = ExecutionQueue::new();
    a::Activity::decode(&call(&s, &mut queue, caller, admitted, a::SCHEDULE)).unwrap();
    let mut restarted = Server::new(Volume::mount(&mut d).unwrap());
    let fresh = grant(&mut restarted, 0, 9, 4, INSPECT_RIGHT | WRITE_RIGHT);
    let mut queue = ExecutionQueue::new();
    let mut d = Deferred::new(d);
    d.signals.release.set(true);
    assert!(
        restarted
            .run_scheduled(&mut d, &mut queue, 1, |_, _, _| 1)
            .is_none()
    );
    assert_eq!(
        restarted.admission_status(fresh, admitted.id, 1).unwrap(),
        admitted
    );
    assert_eq!(
        call(&restarted, &mut queue, fresh, admitted, a::ACTIVITY).status,
        Error::Unavailable as u8
    );
    a::Activity::decode(&call(&restarted, &mut queue, fresh, admitted, a::SCHEDULE)).unwrap();
    assert_eq!(
        restarted
            .run_scheduled(&mut d, &mut queue, 1, |_, _, _| 1)
            .unwrap()
            .unwrap()
            .state,
        State::Committed
    );
}

#[test]
fn two_scheduled_writes_recheck_version_at_each_effect_and_keep_fifo_order() {
    let (mut s, d, caller, request) = base();
    let (d, first) = accept(&mut s, d, caller, request);
    let (d, second) = accept(
        &mut s,
        d,
        caller,
        Replacement {
            retry: Retry {
                key: 43,
                ..request.retry
            },
            ..request
        },
    );
    let mut queue = ExecutionQueue::new();
    for status in [first, second] {
        a::Activity::decode(&call(&s, &mut queue, caller, status, a::SCHEDULE)).unwrap();
    }
    let mut d = Deferred::new(d);
    d.signals.release.set(true);
    assert_eq!(
        s.run_scheduled(&mut d, &mut queue, 1, |_, _, _| 1)
            .unwrap()
            .unwrap()
            .id,
        first.id
    );
    let version = s.volume.stat(request.id).unwrap().version;
    assert_eq!(
        s.run_scheduled(&mut d, &mut queue, 1, |_, _, _| 1)
            .unwrap()
            .unwrap()
            .state,
        State::Cancelled
    );
    assert_eq!(s.volume.stat(request.id).unwrap().version, version);
    assert!(queue.is_empty());
}

#[test]
fn uncertain_device_settlement_abandons_volatile_pending_work_without_replay() {
    let (mut s, d, caller, request) = base();
    let (d, first) = accept(&mut s, d, caller, request);
    let (d, second) = accept(
        &mut s,
        d,
        caller,
        Replacement {
            retry: Retry {
                key: 43,
                ..request.retry
            },
            ..request
        },
    );
    let mut queue = ExecutionQueue::new();
    for status in [first, second] {
        a::Activity::decode(&call(&s, &mut queue, caller, status, a::SCHEDULE)).unwrap();
    }
    let mut d = Deferred::new(d);
    d.signals.release.set(true);
    d.signals.fail.set(true);
    assert_eq!(
        s.run_scheduled(&mut d, &mut queue, 1, |_, _, _| 1).unwrap(),
        Err(Error::Uncertain)
    );
    assert!(queue.is_empty());
    let mut fresh = Server::new(Volume::mount(&mut d.disk).unwrap());
    let fresh_caller = grant(&mut fresh, 0, 9, 4, INSPECT_RIGHT | WRITE_RIGHT);
    for status in [first, second] {
        assert_eq!(
            fresh
                .admission_status(fresh_caller, status.id, 1)
                .unwrap()
                .state,
            State::Admitted
        );
    }
    assert!(
        fresh
            .run_scheduled(&mut d, &mut ExecutionQueue::new(), 1, |_, _, _| 1)
            .is_none()
    );
}
