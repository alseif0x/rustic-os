// SPDX-License-Identifier: Apache-2.0
mod support;
use rustic_abi::files::{operation::*, reference::*, *};
use rustic_file_service::Server;
use support::*;
#[test]
fn scope_helper_identity_expiration_and_regrant_are_enforced() {
    let (mut s, mut d, a, b) = setup();
    let c = grant(&mut s, 0, a, 3, 100);
    let h = grant(&mut s, 1, a, 1, 0);
    assert_eq!(run(&mut s, &mut d, 0, request(READ, a, c), 1).status, 0);
    assert_eq!(
        run(&mut s, &mut d, 0, request(READ, b, c), 1).status,
        Error::Denied as u8
    );
    assert_eq!(
        run(&mut s, &mut d, 1, request(BEGIN, a, h), 1).status,
        Error::Denied as u8
    );
    assert_eq!(
        s.handle(&mut d, 0, 99, request(READ, a, c), 1).status,
        Error::Denied as u8
    );
    assert_eq!(
        run(&mut s, &mut d, 0, request(READ, a, c), 100).status,
        Error::Expired as u8
    );
    s.revoke(0).unwrap();
    assert_eq!(
        run(&mut s, &mut d, 0, request(READ, a, c), 1).status,
        Error::Revoked as u8
    );
    let fresh = grant(&mut s, 0, a, 3, 0);
    assert_ne!(c, fresh);
    assert_eq!(
        run(&mut s, &mut d, 0, request(READ, a, c), 1).status,
        Error::Revoked as u8
    );
    assert_eq!(run(&mut s, &mut d, 0, request(READ, a, fresh), 1).status, 0);
}
#[test]
fn staged_effects_are_bounded_revocable_and_version_checked() {
    let (mut s, mut d, a, b) = setup();
    let c = grant(&mut s, 0, a, 3, 0);
    let other = grant(&mut s, 1, b, 3, 0);
    let third = grant(&mut s, 2, a, 3, 0);
    let version = s.volume.stat(a).unwrap().version;
    let begin = Packet {
        arg: 3,
        version,
        ..request(BEGIN, a, c)
    };
    assert_eq!(run(&mut s, &mut d, 0, begin, 0).status, 0);
    let bv = s.volume.stat(b).unwrap().version;
    assert_eq!(
        run(
            &mut s,
            &mut d,
            1,
            Packet {
                arg: 3,
                version: bv,
                ..request(BEGIN, b, other)
            },
            0
        )
        .status,
        0
    );
    assert_eq!(
        run(
            &mut s,
            &mut d,
            2,
            Packet {
                context: third,
                ..begin
            },
            0
        )
        .status,
        Error::Busy as u8
    );
    assert_eq!(s.pending(), 2);
    let mut chunk = request(CHUNK, a, c);
    chunk.count = 3;
    chunk.data[..3].copy_from_slice(b"new");
    chunk.arg = 1;
    assert_eq!(run(&mut s, &mut d, 0, chunk, 0).status, Error::Offset as u8);
    chunk.arg = 0;
    assert_eq!(run(&mut s, &mut d, 0, chunk, 0).status, 0);
    // Intervening owner edit invalidates the staged expected version.
    s.volume.replace(&mut d, a, version, b"owner").unwrap();
    assert_eq!(
        run(&mut s, &mut d, 0, request(COMMIT, a, c), 0).status,
        Error::Version as u8
    );
    let mut bytes = [0; 5];
    s.volume.read(&mut d, a, 0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"owner");
    s.revoke(1).unwrap();
    assert_eq!(s.pending(), 0);
    assert_eq!(
        run(&mut s, &mut d, 1, request(COMMIT, b, other), 0).status,
        Error::Revoked as u8
    );
    assert_eq!(s.volume.stat(b).unwrap().length, 0);
}
#[test]
fn incomplete_or_expired_transfer_cannot_modify_a_file() {
    let (mut s, mut d, a, _) = setup();
    let c = grant(&mut s, 0, a, 3, 10);
    let version = s.volume.stat(a).unwrap().version;
    assert_eq!(
        run(
            &mut s,
            &mut d,
            0,
            Packet {
                arg: 1,
                version,
                ..request(BEGIN, a, c)
            },
            0
        )
        .status,
        0
    );
    assert_eq!(
        run(&mut s, &mut d, 0, request(COMMIT, a, c), 0).status,
        Error::Offset as u8
    );
    s.expire(10);
    assert_eq!(s.pending(), 0);
    assert_eq!(
        run(&mut s, &mut d, 0, request(COMMIT, a, c), 10).status,
        Error::Expired as u8
    );
    assert_eq!(s.volume.stat(a).unwrap().length, 0);
}
#[test]
fn a_second_scope_adds_one_object_and_is_never_inherited() {
    let (mut s, mut d, a, b) = setup();
    let c = grant(&mut s, 0, a, 3, 0);
    assert_eq!(
        run(&mut s, &mut d, 0, request(READ, b, c), 1).status,
        Error::Denied as u8
    );
    assert_eq!(s.extend(0, c, b), Ok(c));
    assert_eq!(run(&mut s, &mut d, 0, request(READ, a, c), 1).status, 0);
    assert_eq!(run(&mut s, &mut d, 0, request(READ, b, c), 1).status, 0);
    // The same rights apply in both scopes: staging a write on the second object
    // is admitted exactly as on the primary one.
    let version = s.volume.stat(b).unwrap().version;
    assert_eq!(
        run(
            &mut s,
            &mut d,
            0,
            Packet {
                arg: 3,
                version,
                ..request(BEGIN, b, c)
            },
            1
        )
        .status,
        0
    );
    assert_eq!(run(&mut s, &mut d, 0, request(ABORT, b, c), 1).status, 0);
    // Nothing else becomes reachable.
    let third = s
        .volume
        .create(&mut d, 4, b"c", rustic_fs::Kind::File)
        .unwrap()
        .id;
    assert_eq!(
        run(&mut s, &mut d, 0, request(READ, third, c), 1).status,
        Error::Denied as u8
    );
    // A helper derived from the extended root keeps only the primary scope, and it
    // cannot be issued into the second one at all.
    let h = s.derive(1, c, binding(1, a, b, 1, 0), 1).unwrap();
    assert_eq!(run(&mut s, &mut d, 1, request(READ, a, h), 1).status, 0);
    assert_eq!(
        run(&mut s, &mut d, 1, request(READ, b, h), 1).status,
        Error::Denied as u8
    );
    assert_eq!(
        s.derive(2, c, binding(2, b, 0, 1, 0), 1),
        Err(Error::Denied)
    );
}
#[test]
fn an_invalid_second_scope_is_refused_at_installation() {
    let (mut s, _d, a, b) = setup();
    for (scope, second) in [(a, a), (a, 9999), (0, b), (4, a), (a, 4)] {
        assert_eq!(install(&mut s, 0, scope, second, 3, 0), Err(Error::Invalid));
    }
    // A directory is refused even when the two subtrees are disjoint: the second
    // scope reaches one companion object, not a set that can still grow.
    assert_eq!(install(&mut s, 0, 1, 4, 3, 0), Err(Error::Invalid));
    let c = grant(&mut s, 0, a, 3, 0);
    assert_eq!(s.extend(0, c, 4), Err(Error::Invalid));
    // One live file outside the primary scope stays acceptable.
    assert!(install(&mut s, 1, a, b, 3, 0).is_ok());
}

/// Two owner-equivalent clients that retain their own operations are separated by
/// their recovery subject alone: same workspace, same rights, same retry key. The
/// supervisor gives its tasks-owner child the journal object as subject for this
/// reason, so an equal journal version can never resolve the other client's work.
#[test]
fn distinct_subjects_keep_equal_retry_keys_in_separate_namespaces() {
    let (mut s, mut d, a, b) = setup();
    s.volume.enable_recovery(&mut d, LINEAGE).unwrap();
    s.volume.enable_operations(&mut d).unwrap();
    let workspace = Workspace::new(LINEAGE, 4).unwrap();
    let retry = Retry {
        epoch: Epoch::new(1).unwrap(),
        key: Key::new(4).unwrap(),
    };
    // The shell's owner client keeps subject 1; a tasks-owner child is identified
    // by the journal object it was granted.
    let shell = recording(&mut s, 0, a, 1).unwrap();
    let owner = recording(&mut s, 1, b, u64::from(b)).unwrap();
    let shell_plan = plan(&s, workspace, a, retry);
    assert_eq!(publish(&mut s, &mut d, 0, shell, shell_plan, b"shell"), 0);
    // The same tuple under another subject is a new operation: it is neither
    // refused as a conflicting retry nor replayed as the other client's effect.
    let owner_plan = plan(&s, workspace, b, retry);
    assert_eq!(publish(&mut s, &mut d, 1, owner, owner_plan, b"owner"), 0);
    let mine = run(&mut s, &mut d, 0, retried(workspace, retry, shell), 1);
    let theirs = run(&mut s, &mut d, 1, retried(workspace, retry, owner), 1);
    assert_eq!((mine.status, theirs.status), (0, 0));
    assert_ne!(
        mine.version, theirs.version,
        "one record answered both subjects"
    );
    assert_eq!(s.volume.stat(a).unwrap().length, 5);
    assert_eq!(s.volume.stat(b).unwrap().length, 5);
    // Neither client can resolve the other's operation, by retry tuple or by a
    // guessed identifier; both answers are indistinguishable from a missing one.
    for (slot, context, other) in [(0, shell, theirs.version), (1, owner, mine.version)] {
        let stranger = Lookup::Id(OperationId::new(LINEAGE, other).unwrap());
        assert_eq!(
            run(&mut s, &mut d, slot, stranger.packet(context), 1).status,
            Error::OutcomeUnknown as u8
        );
    }
    // Each still resolves its own by identifier, under its own subject.
    for (slot, context, own) in [(0, shell, mine.version), (1, owner, theirs.version)] {
        let mine = Lookup::Id(OperationId::new(LINEAGE, own).unwrap());
        assert_eq!(run(&mut s, &mut d, slot, mine.packet(context), 1).status, 0);
    }
}
/// Recovery lineage of the volume these operations are recorded in.
const LINEAGE: [u8; 16] = [3; 16];
/// One replacement of `object` under the given retry tuple, at its live version.
fn plan(s: &Server, workspace: Workspace, object: u32, retry: Retry) -> Replacement {
    Replacement {
        workspace,
        resource: Resource::new(workspace, object).unwrap(),
        expected_version: Version::new(s.volume.stat(object).unwrap().version).unwrap(),
        retry,
    }
}
/// Stage and commit one bounded replacement; the result is the commit status.
fn publish(
    s: &mut Server,
    d: &mut Memory,
    slot: usize,
    context: u32,
    r: Replacement,
    bytes: &[u8],
) -> u8 {
    let object = r.resource.object();
    assert_eq!(
        run(s, d, slot, r.packet(bytes.len(), context).unwrap(), 1).status,
        0
    );
    let mut chunk = request(REPLACE_CHUNK, object, context);
    chunk.count = bytes.len() as u8;
    chunk.data[..bytes.len()].copy_from_slice(bytes);
    assert_eq!(run(s, d, slot, chunk, 1).status, 0);
    run(s, d, slot, request(REPLACE_COMMIT, object, context), 1).status
}
fn retried(workspace: Workspace, retry: Retry, context: u32) -> Packet {
    Lookup::Retry { workspace, retry }.packet(context)
}
#[test]
fn extension_names_one_live_root_once_and_is_fenced_with_it() {
    let (mut s, mut d, a, b) = setup();
    let c = grant(&mut s, 0, a, 3, 100);
    assert_eq!(s.extend(0, c + 1, b), Err(Error::Denied));
    assert_eq!(s.extend(1, c, b), Err(Error::Denied));
    assert_eq!(s.extend(0, c, 0), Err(Error::Denied));
    assert_eq!(s.extend(0, c, a), Err(Error::Invalid));
    assert_eq!(s.extend(0, c, b), Ok(c));
    assert_eq!(s.extend(0, c, b), Err(Error::Denied));
    // A derived helper is attenuated to its parent and cannot be extended.
    let h = s.derive(1, c, binding(1, a, 0, 1, 100), 1).unwrap();
    assert_eq!(s.extend(1, h, b), Err(Error::Denied));
    // One deadline and one revocation cover both scopes.
    assert_eq!(
        run(&mut s, &mut d, 0, request(READ, b, c), 100).status,
        Error::Expired as u8
    );
    assert_eq!(run(&mut s, &mut d, 0, request(READ, b, c), 1).status, 0);
    s.revoke(0).unwrap();
    for id in [a, b] {
        assert_eq!(
            run(&mut s, &mut d, 0, request(READ, id, c), 1).status,
            Error::Revoked as u8
        );
    }
    assert_eq!(s.extend(0, c, b), Err(Error::Revoked));
    // A fresh root on the same slot starts without a second scope.
    let fresh = grant(&mut s, 0, a, 3, 0);
    assert_eq!(run(&mut s, &mut d, 0, request(READ, a, fresh), 1).status, 0);
    assert_eq!(
        run(&mut s, &mut d, 0, request(READ, b, fresh), 1).status,
        Error::Denied as u8
    );
}
