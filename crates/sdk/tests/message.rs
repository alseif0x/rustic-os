// SPDX-License-Identifier: Apache-2.0
use rustic_sdk::{Error, abi::ipc, ipc::Message};
#[test]
fn outbound_has_canonical_header_and_no_claimed_sender() {
    let message = Message::new(0x0102030405060708, &[42; 64]).unwrap();
    assert_eq!(message.wire().len(), 88);
    assert_eq!(&message.wire()[..8], &[1, 0, 1, 0, 64, 0, 0, 0]);
    assert_eq!(&message.wire()[8..16], &[8, 7, 6, 5, 4, 3, 2, 1]);
    assert_eq!(message.sender(), 0);
    assert_eq!(
        Message::new(0, &[0; 65]).err(),
        Some(Error::Ipc(ipc::Error::Size))
    );
}
#[test]
fn inbound_preserves_sender_correlation_and_payload() {
    let mut bytes = [0; 27];
    bytes.copy_from_slice(Message::new(19, b"abc").unwrap().wire());
    bytes[16..24].copy_from_slice(&7u64.to_le_bytes());
    let message = Message::from_received(&bytes).unwrap();
    assert_eq!(message.correlation(), 19);
    assert_eq!(message.sender(), 7);
    assert_eq!(message.payload(), b"abc");
    for length in 0..27 {
        assert!(Message::from_received(&bytes[..length]).is_err());
    }
    for offset in [0, 2, 4] {
        let mut bad = bytes;
        bad[offset] = 255;
        assert!(Message::from_received(&bad).is_err());
    }
    bytes[16..24].fill(0);
    assert!(Message::from_received(&bytes).is_err());
}
