// SPDX-License-Identifier: Apache-2.0
use super::super::*;
use rustic_abi::files::{
    self as f,
    admission::{self as a, Status},
    operation, reference,
};

fn logical(r: Replacement) -> operation::Replacement {
    let workspace = reference::Workspace::new(r.retry.lineage, r.workspace).unwrap();
    operation::Replacement {
        workspace,
        resource: reference::Resource::new(workspace, r.id).unwrap(),
        expected_version: reference::Version::new(r.version).unwrap(),
        retry: operation::Retry {
            epoch: reference::Epoch::new(r.retry.epoch).unwrap(),
            key: operation::Key::new(r.retry.key).unwrap(),
        },
    }
}
fn call(s: &mut Server, d: &mut Memory, c: Caller, mut p: f::Packet) -> Result<f::Packet, Error> {
    p.context = c.context;
    let p = f::Packet::decode(&p.encode()).unwrap();
    let r = s.handle(d, c.slot, c.peer, p, 1);
    f::Packet::decode(&r.encode())
        .unwrap()
        .checked_reply(p.op, c.context)
}
fn stage(s: &mut Server, d: &mut Memory, c: Caller, r: Replacement) {
    let mut p = logical(r).packet(5, c.context).unwrap();
    p.op = a::OPEN;
    call(s, d, c, p).unwrap();
    let mut p = f::Packet::new(a::CHUNK);
    p.id = r.id;
    p.count = 5;
    p.data[..5].copy_from_slice(b"after");
    call(s, d, c, p).unwrap();
}
fn submit(s: &mut Server, d: &mut Memory, c: Caller, r: Replacement) -> Status {
    let mut p = f::Packet::new(a::ACCEPT);
    p.id = r.id;
    Status::decode(&call(s, d, c, p).unwrap()).unwrap()
}

#[test]
fn public_acceptance_retry_remount_execute_and_receipt_preserve_identity() {
    let (mut s, mut d, c, r) = base();
    stage(&mut s, &mut d, c, r);
    let accepted = submit(&mut s, &mut d, c, r);
    assert_eq!(accepted.state, a::State::Admitted);
    assert_eq!(s.volume.stat(r.id).unwrap().version, r.version);
    let mut retry = operation::Lookup::Retry {
        workspace: logical(r).workspace,
        retry: logical(r).retry,
    }
    .packet(c.context);
    retry.op = a::RETRY;
    assert_eq!(
        Status::decode(&call(&mut s, &mut d, c, retry).unwrap()).unwrap(),
        accepted
    );
    let mut s = Server::new(Volume::mount(&mut d).unwrap());
    let c = grant(&mut s, 0, 9, 4, INSPECT_RIGHT);
    stage(&mut s, &mut d, c, r);
    assert_eq!(submit(&mut s, &mut d, c, r), accepted);
    let execute = accepted.id.packet(a::EXECUTE, c.context).unwrap();
    assert_eq!(call(&mut s, &mut d, c, execute).unwrap_err(), Error::Denied);
    let c = grant(&mut s, 0, 9, 4, INSPECT_RIGHT | WRITE_RIGHT);
    let committed = Status::decode(&call(&mut s, &mut d, c, execute).unwrap()).unwrap();
    assert_eq!(committed.state, a::State::Committed);
    let seq = s.volume.sequence();
    assert_eq!(
        Status::decode(&call(&mut s, &mut d, c, execute).unwrap()).unwrap(),
        committed
    );
    assert_eq!(s.volume.sequence(), seq);
    let p = operation::Lookup::Id(committed.completion().unwrap()).packet(c.context);
    let receipt = call(&mut s, &mut d, c, p).unwrap();
    assert_eq!(receipt.version, committed.terminal);
}

#[test]
fn cancel_only_cannot_inspect_write_read_or_cross_scope_but_prevents_execution() {
    let (mut s, mut d, c, r) = base();
    stage(&mut s, &mut d, c, r);
    let accepted = submit(&mut s, &mut d, c, r);
    let cancel = accepted.id.packet(a::CANCEL, 0).unwrap();
    assert_eq!(call(&mut s, &mut d, c, cancel).unwrap_err(), Error::Denied);
    let other = s.volume.lookup(4, b"b").unwrap().id;
    for (subject, scope) in [(10, 4), (9, other)] {
        let c = grant(&mut s, 1, subject, scope, f::CANCEL_RIGHT);
        assert_eq!(
            call(&mut s, &mut d, c, cancel).unwrap_err(),
            Error::OutcomeUnknown
        );
    }
    let only = grant(&mut s, 1, 9, r.id, f::CANCEL_RIGHT);
    for op in [a::GET, a::EXECUTE] {
        assert_eq!(
            call(&mut s, &mut d, only, accepted.id.packet(op, 0).unwrap()).unwrap_err(),
            Error::Denied
        );
    }
    let mut read = f::Packet::new(f::READ);
    read.id = r.id;
    assert_eq!(call(&mut s, &mut d, only, read).unwrap_err(), Error::Denied);
    let cancelled = Status::decode(&call(&mut s, &mut d, only, cancel).unwrap()).unwrap();
    assert_eq!(cancelled.state, a::State::Cancelled);
    let seq = s.volume.sequence();
    assert_eq!(
        Status::decode(&call(&mut s, &mut d, only, cancel).unwrap()).unwrap(),
        cancelled
    );
    assert_eq!(
        Status::decode(
            &call(
                &mut s,
                &mut d,
                c,
                accepted.id.packet(a::EXECUTE, 0).unwrap()
            )
            .unwrap()
        )
        .unwrap(),
        cancelled
    );
    assert_eq!(s.volume.sequence(), seq);
    assert_eq!(s.volume.stat(r.id).unwrap().version, r.version);
}

#[test]
fn staging_profiles_cannot_consume_or_abort_each_other() {
    for admitted in [true, false] {
        let (mut s, mut d, c, r) = base();
        let mut p = logical(r).packet(0, c.context).unwrap();
        if admitted {
            p.op = a::OPEN;
        }
        call(&mut s, &mut d, c, p).unwrap();
        for op in if admitted {
            [f::REPLACE_COMMIT, f::REPLACE_ABORT, f::COMMIT, f::ABORT]
        } else {
            [a::ACCEPT, a::ABORT, a::CHUNK, a::ACCEPT]
        } {
            let mut p = f::Packet::new(op);
            p.id = r.id;
            if matches!(op, f::CHUNK | a::CHUNK) {
                p.count = 1;
            }
            assert!(call(&mut s, &mut d, c, p).is_err());
            assert_eq!(s.pending(), 1);
        }
        let mut p = f::Packet::new(if admitted {
            a::ACCEPT
        } else {
            f::REPLACE_COMMIT
        });
        p.id = r.id;
        call(&mut s, &mut d, c, p).unwrap();
        assert_eq!(s.pending(), 0);
    }
}

#[test]
fn public_cancel_losing_authority_drains_every_pending_command_and_reports_truth() {
    for cut in 1..=14 {
        let (mut s, d, c, r) = base();
        let (d, accepted) = accept(&mut s, d, c, r);
        let c = grant(&mut s, 1, 9, 4, f::CANCEL_RIGHT);
        let mut d = Deferred::new(d);
        let signals = d.signals.clone();
        let mut waits = 0;
        let result = s.cancel_admission_with(&mut d, c, accepted.id, |clients, pending| {
            if pending && signals.submitted.get() == cut {
                waits += 1;
                clients.revoke(c.slot).unwrap();
                signals.release.set(waits >= 3);
            } else {
                signals.release.set(true);
            }
            1
        });
        let late = cut >= 13;
        assert_eq!(
            result,
            Err(if late {
                Error::Uncertain
            } else {
                Error::Revoked
            })
        );
        assert_eq!(signals.submitted.get(), signals.settled.get());
        let v = Volume::mount(&mut d.disk).unwrap();
        assert_eq!(
            v.admission_by_id(9, accepted.id).unwrap().status.state,
            if late {
                State::Cancelled
            } else {
                State::Admitted
            }
        );
        assert_eq!(v.stat(r.id).unwrap().version, r.version);
    }
}
