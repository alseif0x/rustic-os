// SPDX-License-Identifier: Apache-2.0
#[path = "receipts/rotation.rs"]
mod rotation;
mod support;
use rustic_abi::files::{
    recovery::{Receipt, Retry},
    *,
};
use rustic_file_service::{Grant, Server};
use rustic_fs::Volume;
use support::*;
fn authorize(
    s: &mut Server,
    slot: usize,
    scope: u32,
    rights: u8,
    subject: u64,
    expires: u64,
) -> u32 {
    s.grant(
        slot,
        Grant {
            peer: slot as u64 + 10,
            endpoint: slot as u64 + 1,
            scope,
            rights,
            subject,
            expires,
            generation: 0,
        },
    )
    .unwrap()
}
fn begin(id: u32, context: u32, version: u64, retry: Retry) -> Packet {
    let mut p = request(TRACK_BEGIN, id, context);
    p.count = 32;
    p.data[..32].copy_from_slice(&retry.encode());
    p.version = version;
    p.arg = 3;
    p
}
fn lookup(id: u32, context: u32, retry: Retry) -> Packet {
    let mut p = request(RECEIPT, id, context);
    p.count = 32;
    p.data[..32].copy_from_slice(&retry.encode());
    p
}
fn stage(s: &mut Server, d: &mut Memory, id: u32, context: u32, version: u64, retry: Retry) {
    assert_eq!(
        run(s, d, 0, begin(id, context, version, retry), 1).status,
        0
    );
    let mut chunk = request(CHUNK, id, context);
    chunk.count = 3;
    chunk.data[..3].copy_from_slice(b"new");
    assert_eq!(run(s, d, 0, chunk, 1).status, 0);
}
#[test]
fn subjects_scopes_expiry_revoke_and_inspection_only_replay_survive_restart() {
    let (mut s, mut d, a, b) = setup();
    s.volume.enable_recovery(&mut d, [7; 16]).unwrap();
    let key = Retry {
        lineage: [7; 16],
        epoch: 1,
        key: 99,
    };
    let version = s.volume.stat(a).unwrap().version;
    let c = authorize(&mut s, 0, a, 7, 9, 0);
    let h = authorize(&mut s, 1, a, 5, 8, 0);
    assert_eq!(
        run(&mut s, &mut d, 0, begin(b, c, version, key), 1).status,
        Error::Denied as u8
    );
    stage(&mut s, &mut d, a, c, version, key);
    s.revoke(0).unwrap();
    assert_eq!(
        run(&mut s, &mut d, 0, request(COMMIT, a, c), 1).status,
        Error::Revoked as u8
    );
    assert_eq!(s.volume.stat(a).unwrap().version, version);
    let c = authorize(&mut s, 0, a, 7, 9, 2);
    stage(&mut s, &mut d, a, c, version, key);
    assert_eq!(
        run(&mut s, &mut d, 0, request(COMMIT, a, c), 2).status,
        Error::Expired as u8
    );
    let c = authorize(&mut s, 0, a, 7, 9, 0);
    stage(&mut s, &mut d, a, c, version, key);
    let receipt = Receipt::decode(run(&mut s, &mut d, 0, request(COMMIT, a, c), 1)).unwrap();
    assert_eq!(
        run(&mut s, &mut d, 1, lookup(a, h, key), 1).status,
        Error::OutcomeUnknown as u8
    );
    assert_eq!(
        s.handle(&mut d, 0, 88, lookup(a, c, key), 1).status,
        Error::Denied as u8
    );
    s.revoke(0).unwrap();
    assert_eq!(
        run(&mut s, &mut d, 0, lookup(a, c, key), 1).status,
        Error::Revoked as u8
    );
    let mut s = Server::new(Volume::mount(&mut d).unwrap());
    assert_eq!(
        run(&mut s, &mut d, 0, lookup(a, c, key), 1).status,
        Error::Denied as u8
    );
    let c = authorize(&mut s, 0, a, INSPECT_RIGHT, 9, 0);
    stage(&mut s, &mut d, a, c, version, key);
    assert_eq!(
        Receipt::decode(run(&mut s, &mut d, 0, request(COMMIT, a, c), 1)),
        Ok(receipt)
    );
    assert_eq!(
        run(
            &mut s,
            &mut d,
            0,
            begin(a, c, receipt.committed, Retry { key: 100, ..key }),
            1
        )
        .status,
        Error::Denied as u8
    );
    assert_eq!(
        run(&mut s, &mut d, 0, request(READ, a, c), 1).status,
        Error::Denied as u8
    );
    assert_eq!(
        run(&mut s, &mut d, 0, lookup(b, c, key), 1).status,
        Error::Denied as u8
    );
}
#[test]
fn retained_receipt_cannot_turn_into_write_when_epoch_rotates_during_staging() {
    let (mut s, mut d, a, _) = setup();
    s.volume.enable_recovery(&mut d, [7; 16]).unwrap();
    let key = Retry {
        lineage: [7; 16],
        epoch: 1,
        key: 99,
    };
    let version = s.volume.stat(a).unwrap().version;
    let c = authorize(&mut s, 0, a, 7, 9, 0);
    stage(&mut s, &mut d, a, c, version, key);
    assert_eq!(run(&mut s, &mut d, 0, request(COMMIT, a, c), 1).status, 0);
    let committed = s.volume.stat(a).unwrap().version;
    let c = authorize(&mut s, 0, a, INSPECT_RIGHT, 9, 0);
    stage(&mut s, &mut d, a, c, version, key);
    s.volume.advance_epoch(&mut d).unwrap();
    assert_eq!(
        run(&mut s, &mut d, 0, request(COMMIT, a, c), 1).status,
        Error::ExpiredEpoch as u8
    );
    assert_eq!(s.volume.stat(a).unwrap().version, committed);
}

#[test]
fn retained_target_authority_is_checked_before_replay_disclosure() {
    let (mut s, mut d, a, b) = setup();
    s.volume.enable_recovery(&mut d, [7; 16]).unwrap();
    let key = Retry {
        lineage: [7; 16],
        epoch: 1,
        key: 99,
    };
    let version = s.volume.stat(b).unwrap().version;
    let c = authorize(&mut s, 0, 0, 7, 9, 0);
    stage(&mut s, &mut d, b, c, version, key);
    assert_eq!(run(&mut s, &mut d, 0, request(COMMIT, b, c), 1).status, 0);
    let c = authorize(&mut s, 0, a, 7, 9, 0);
    let version = s.volume.stat(a).unwrap().version;
    assert_eq!(
        run(&mut s, &mut d, 0, begin(a, c, version, key), 1).status,
        Error::Denied as u8
    );
    assert_eq!(
        run(&mut s, &mut d, 0, lookup(a, c, key), 1).status,
        Error::OutcomeUnknown as u8
    );
}
