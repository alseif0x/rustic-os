// SPDX-License-Identifier: Apache-2.0
mod support;
use rustic_abi::files::*;
use rustic_file_service::Grant;
use support::*;
fn helper(scope: u32, rights: u8, expires: u64) -> Grant {
    Grant {
        peer: 11,
        endpoint: 2,
        scope,
        rights,
        expires,
        generation: 0,
        subject: 999,
    }
}
#[test]
fn helper_is_a_live_subset_and_cannot_delegate_or_select_recovery_subject() {
    let (mut s, mut d, a, b) = setup();
    let c = grant(&mut s, 0, a, 3, 100);
    for invalid in [
        helper(b, 1, 100),
        helper(a, 7, 100),
        helper(a, 1, 0),
        helper(a, 1, 101),
    ] {
        assert_eq!(s.derive(1, c, invalid, 1), Err(Error::Denied));
    }
    let h = s.derive(1, c, helper(a, 1, 90), 1).unwrap();
    assert_eq!(s.grant_at(1).unwrap().subject, 0);
    assert_eq!(run(&mut s, &mut d, 1, request(READ, a, h), 1).status, 0);
    assert_eq!(
        run(&mut s, &mut d, 1, request(BEGIN, a, h), 1).status,
        Error::Denied as u8
    );
    assert_eq!(s.derive(2, h, helper(a, 1, 80), 1), Err(Error::Denied));
    assert_eq!(s.derive(2, c, helper(a, 1, 100), 100), Err(Error::Expired));
    assert_eq!(
        run(&mut s, &mut d, 1, request(READ, a, h), 90).status,
        Error::Expired as u8
    );
}
#[test]
fn session_fence_clears_staging_and_keeps_owner_authority() {
    let (mut s, mut d, a, _) = setup();
    let c = grant(&mut s, 0, a, 3, 0);
    let h = s.derive(1, c, helper(a, 1, 0), 0).unwrap();
    let owner = grant(&mut s, 2, 0, 3, 0);
    let begin = Packet {
        version: s.volume.stat(a).unwrap().version,
        arg: 1,
        ..request(BEGIN, a, c)
    };
    assert_eq!(run(&mut s, &mut d, 0, begin, 0).status, 0);
    assert_eq!(s.pending(), 1);
    assert_eq!(s.revoke(1), Ok(3)); // Either member identifies the same root.
    assert_eq!(s.pending(), 0);
    assert_eq!(
        run(&mut s, &mut d, 0, request(COMMIT, a, c), 0).status,
        Error::Revoked as u8
    );
    assert_eq!(
        run(&mut s, &mut d, 1, request(READ, a, h), 0).status,
        Error::Revoked as u8
    );
    assert_eq!(run(&mut s, &mut d, 2, request(READ, a, owner), 0).status, 0);
    assert_eq!(s.derive(3, c, helper(a, 1, 0), 0), Err(Error::Revoked));
    assert_eq!(s.volume.stat(a).unwrap().length, 0);
}
#[test]
fn root_death_replacement_and_moved_peer_never_revive_a_helper() {
    let (mut s, mut d, a, _) = setup();
    let c = grant(&mut s, 0, a, 3, 0);
    let h = s.derive(1, c, helper(a, 1, 0), 0).unwrap();
    // A moved endpoint keeps the service slot, which still authenticates the original PID.
    assert_eq!(
        s.handle(&mut d, 0, 11, request(READ, a, c), 0).status,
        Error::Denied as u8
    );
    s.detach(0);
    assert_eq!(
        run(&mut s, &mut d, 1, request(READ, a, h), 0).status,
        Error::Revoked as u8
    );
    let fresh = grant(&mut s, 0, a, 3, 0);
    assert_ne!(c, fresh);
    assert_eq!(
        run(&mut s, &mut d, 0, request(READ, a, c), 0).status,
        Error::Revoked as u8
    );
    assert_eq!(
        run(&mut s, &mut d, 1, request(READ, a, h), 0).status,
        Error::Revoked as u8
    );
    let h2 = s.derive(1, fresh, helper(a, 1, 0), 0).unwrap();
    let _new = grant(&mut s, 0, a, 3, 0);
    assert_eq!(
        run(&mut s, &mut d, 1, request(READ, a, h2), 0).status,
        Error::Revoked as u8
    );
}
