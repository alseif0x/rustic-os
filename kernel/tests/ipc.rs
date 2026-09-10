// SPDX-License-Identifier: Apache-2.0
use rustic_kernel::ipc::{ALL, Broker, Error, Message, READ, TRANSFER, WRITE};

fn packet(correlation: u64) -> [u8; 32] {
    let mut bytes = [0; 32];
    bytes[..2].copy_from_slice(&1u16.to_le_bytes());
    bytes[2..4].copy_from_slice(&1u16.to_le_bytes());
    bytes[4..8].copy_from_slice(&8u32.to_le_bytes());
    bytes[8..16].copy_from_slice(&correlation.to_le_bytes());
    bytes[24..32].copy_from_slice(&42u64.to_le_bytes());
    bytes
}

#[test]
fn empty_and_maximum_payloads_round_trip_without_truncation() {
    for length in [0usize, 64] {
        let mut bytes = vec![0; 24 + length];
        bytes[..2].copy_from_slice(&1u16.to_le_bytes());
        bytes[2..4].copy_from_slice(&1u16.to_le_bytes());
        bytes[4..8].copy_from_slice(&(length as u32).to_le_bytes());
        bytes[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        bytes[24..].fill(0xa5);
        let message = Message::decode(&bytes, 123).unwrap();
        let output = message.encode();
        assert_eq!(message.length(), 24 + length);
        assert_eq!(&output[24..message.length()], &bytes[24..]);
        assert_eq!(&output[8..16], &u64::MAX.to_le_bytes());
    }
}

#[test]
fn wire_preserves_data_and_kernel_supplies_identity_without_padding_leaks() {
    let message = Message::decode(&packet(123), 77).unwrap();
    let wire = message.encode();
    assert_eq!(&wire[..16], &packet(123)[..16]);
    assert_eq!(&wire[16..24], &77u64.to_le_bytes());
    assert_eq!(&wire[24..32], &42u64.to_le_bytes());
    assert!(wire[32..].iter().all(|b| *b == 0));
    assert_eq!(message.length(), 32);
}

#[test]
fn malformed_packets_cannot_change_queues_or_spoof_identity() {
    let mut broker = Broker::new();
    let (a, b) = broker.connect(1, 2).unwrap();
    for (offset, value, expected) in [
        (0, 2, Error::Version),
        (2, 2, Error::Message),
        (4, 9, Error::Size),
        (16, 1, Error::Message),
    ] {
        let mut bytes = packet(0);
        bytes[offset] = value;
        assert_eq!(broker.send(1, a, &bytes), Err(expected));
        assert_eq!(broker.peek(2, b), Err(Error::WouldBlock));
    }
    for length in [0, 23, 31] {
        assert_eq!(broker.send(1, a, &packet(0)[..length]), Err(Error::Size));
    }
    assert_eq!(Message::decode(&[0; 89], 1), Err(Error::Size));
}

#[test]
fn bounded_fifo_backpressure_and_close_drain_are_lossless() {
    let mut broker = Broker::new();
    let (a, b) = broker.connect(1, 2).unwrap();
    broker.send(1, a, &packet(10)).unwrap();
    broker.send(1, a, &packet(11)).unwrap();
    assert_eq!(broker.send(1, a, &packet(12)), Err(Error::WouldBlock));
    broker.close_owner(1);
    assert_eq!(broker.send(2, b, &packet(0)), Err(Error::Closed));
    for correlation in [10u64, 11] {
        let first = broker.peek(2, b).unwrap();
        assert_eq!(broker.peek(2, b), Ok(first), "peek cannot consume");
        assert_eq!(&first.encode()[8..16], &correlation.to_le_bytes());
        broker.consume(2, b).unwrap();
    }
    assert_eq!(broker.peek(2, b), Err(Error::Closed));
    broker.close_owner(2);
    assert_eq!(broker.counts(), (0, 0));
}

#[test]
fn foreign_stale_and_transferred_handles_cannot_gain_rights() {
    let mut broker = Broker::new();
    let (old, b) = broker.connect(1, 2).unwrap();
    assert_eq!(broker.send(2, old, &packet(0)), Err(Error::Handle));
    assert_eq!(broker.close(2, old), Err(Error::Handle));
    assert_eq!(broker.transfer(1, old, 3, 0xff), Err(Error::Denied));
    broker.check(1, old, WRITE).unwrap();
    broker.send(2, b, &packet(7)).unwrap();
    let moved = broker.transfer(1, old, 3, READ).unwrap();
    assert_ne!(old, moved);
    assert_eq!(broker.check(1, old, READ), Err(Error::Handle));
    assert_eq!(broker.check(1, moved, READ), Err(Error::Handle));
    assert_eq!(broker.check(3, moved, WRITE), Err(Error::Denied));
    assert_eq!(broker.check(3, moved, TRANSFER), Err(Error::Denied));
    assert_eq!(broker.transfer(3, moved, 1, ALL), Err(Error::Denied));
    assert_eq!(
        &broker.peek(3, moved).unwrap().encode()[16..24],
        &2u64.to_le_bytes()
    );
    broker.close_owner(1); // Former owner's death cannot revoke a moved endpoint.
    assert!(broker.peek(3, moved).is_ok());
    broker.close_owner(2);
    broker.close_owner(3);
    assert_eq!(broker.counts(), (0, 0));
}

#[test]
fn bounded_channels_recover_without_reusing_tokens() {
    let mut broker = Broker::new();
    let (old, _) = broker.connect(1, 1).unwrap();
    for _ in 1..rustic_kernel::ipc::CHANNELS {
        broker.connect(1, 1).unwrap();
    }
    assert_eq!(broker.connect(2, 3), Err(Error::Quota));
    assert_eq!(
        broker.counts(),
        (
            rustic_kernel::ipc::CHANNELS,
            2 * rustic_kernel::ipc::CHANNELS
        )
    );
    broker.close_owner(1);
    for _ in 0..1000 {
        let (new, _) = broker.connect(1, 2).unwrap();
        assert!(new > old);
        assert_eq!(broker.check(1, old, READ), Err(Error::Handle));
        broker.close_owner(1);
        broker.close_owner(2);
        assert_eq!(broker.counts(), (0, 0));
    }
}

#[test]
fn arbitrary_wire_headers_never_panic_or_admit_unknown_fields() {
    for length in 0..=90 {
        let bytes = vec![255; length];
        assert!(Message::decode(&bytes, 1).is_err());
    }
    for index in 0..24 {
        let mut bytes = packet(0);
        bytes[index] ^= 0xff;
        let _ = Message::decode(&bytes, 1);
    }
}
