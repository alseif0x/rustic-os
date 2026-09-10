// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{
    recovery::{Receipt, Retry},
    *,
};
#[test]
fn native_receipt_layout_has_explicit_fixed_width_fields() {
    let retry = Retry {
        lineage: [0xa5; 16],
        epoch: 0x0102030405060708,
        key: 9,
    };
    let receipt = Receipt {
        retry,
        id: 42,
        previous: 7,
        committed: 11,
        length: 3,
    };
    let packet = receipt.packet(Packet::new(COMMIT));
    let wire = packet.encode();
    assert_eq!(&wire[..4], &[1, 9, 0, 40]);
    assert_eq!(&wire[4..12], &[42, 0, 0, 0, 3, 0, 0, 0]);
    assert_eq!(&wire[16..24], &11u64.to_le_bytes());
    assert_eq!(&wire[24..40], &[0xa5; 16]);
    assert_eq!(&wire[40..48], &[8, 7, 6, 5, 4, 3, 2, 1]);
    assert_eq!(&wire[48..56], &9u64.to_le_bytes());
    assert_eq!(&wire[56..64], &7u64.to_le_bytes());
    assert_eq!(Receipt::decode(Packet::decode(&wire).unwrap()), Ok(receipt));
}
#[test]
fn malformed_tokens_receipts_and_packet_tails_are_rejected() {
    assert_eq!(Retry::decode(&[0; 31]), Err(Error::Protocol));
    assert_eq!(Retry::decode(&[0; 32]), Err(Error::Invalid));
    let retry = Retry {
        lineage: [1; 16],
        epoch: 1,
        key: 1,
    };
    let mut p = Receipt {
        retry,
        id: 5,
        previous: 1,
        committed: 2,
        length: 0,
    }
    .packet(Packet::new(RECEIPT));
    p.version = 1;
    assert_eq!(Receipt::decode(p), Err(Error::Protocol));
    p.version = 2;
    p.arg = 1025;
    assert_eq!(Receipt::decode(p), Err(Error::Protocol));
    let mut p = Packet::new(RECOVERY);
    p.count = 0;
    p.data[0] = 1;
    assert_eq!(Packet::decode(&p.encode()), Err(Error::Protocol));
    let mut wire = Packet::new(RECOVERY).encode();
    wire[0] = 2;
    assert_eq!(Packet::decode(&wire), Err(Error::Protocol));
}
