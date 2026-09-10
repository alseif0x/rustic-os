// SPDX-License-Identifier: Apache-2.0
mod support;
use rustic_abi::files::*;
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
