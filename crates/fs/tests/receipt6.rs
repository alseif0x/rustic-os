// SPDX-License-Identifier: Apache-2.0
//! Host tests for the v6 receipt table: bounded retention that never evicts an
//! unresolved outcome silently (#51).
use rustic_fs::{Error, RECEIPT_BYTES, RETAINED_V6, Receipt6, Receipts6, Retry};

const LINEAGE: [u8; 16] = [7; 16];

fn receipt(epoch: u64, key: u64) -> Receipt6 {
    Receipt6 {
        retry: Retry {
            lineage: LINEAGE,
            epoch,
            key,
        },
        id: 5,
        previous: 3,
        committed: 4,
        length: 200_000,
    }
}

#[test]
fn eight_receipts_are_retained_and_the_ninth_is_full_not_an_eviction() {
    let mut table = Receipts6::new(LINEAGE).unwrap();
    assert!(table.is_empty());
    for key in 1..=RETAINED_V6 as u64 {
        table.retain(receipt(1, key)).unwrap();
    }
    assert_eq!(table.len(), RETAINED_V6);
    // The table is full: a new identity is refused and nothing is dropped.
    let before = table.checksum();
    assert_eq!(table.retain(receipt(1, 99)), Err(Error::Full));
    assert_eq!(table.len(), RETAINED_V6);
    assert_eq!(table.checksum(), before);
    // Every retained outcome is still answerable after the refusal.
    for key in 1..=RETAINED_V6 as u64 {
        assert!(table.find(receipt(1, key).retry).unwrap().is_some());
    }
    // Repeating a retained identity is idempotent, not a second record.
    table.retain(receipt(1, 1)).unwrap();
    assert_eq!(table.len(), RETAINED_V6);
    assert_eq!(table.checksum(), before);
}

#[test]
fn identity_mismatches_are_refusals_and_a_stale_epoch_is_expired() {
    let mut table = Receipts6::new(LINEAGE).unwrap();
    table.retain(receipt(1, 1)).unwrap();
    // A retained identity is found (this is what makes a retry safe).
    assert_eq!(
        table.find(receipt(1, 1).retry).unwrap(),
        Some(&receipt(1, 1))
    );
    // A new identity in the current epoch is not an error: it is simply new.
    assert_eq!(table.find(receipt(1, 2).retry).unwrap(), None);
    // A different lineage or a zero key is refused.
    let mut other = receipt(1, 3);
    other.retry.lineage = [8; 16];
    assert_eq!(table.find(other.retry), Err(Error::Lineage));
    assert_eq!(table.retain(other), Err(Error::Lineage));
    let mut zero = receipt(1, 0);
    zero.retry.key = 0;
    assert_eq!(table.find(zero.retry), Err(Error::Invalid));
    // An epoch that is not current cannot be looked up or retained.
    assert_eq!(table.find(receipt(2, 4).retry), Err(Error::ExpiredEpoch));
    assert_eq!(table.retain(receipt(2, 5)), Err(Error::ExpiredEpoch));
}

#[test]
fn an_epoch_cannot_rotate_while_a_retained_outcome_would_be_dropped() {
    let mut table = Receipts6::new(LINEAGE).unwrap();
    table.retain(receipt(1, 1)).unwrap();
    // Rotation is refused while evidence is held: dropping it would make an
    // unresolved effect look like it never happened.
    assert_eq!(table.roll_epoch(2), Err(Error::Full));
    assert_eq!(table.epoch(), 1);
    assert_eq!(table.len(), 1);
    // A backwards or equal epoch is invalid, not a rotation.
    assert_eq!(table.roll_epoch(1), Err(Error::Invalid));

    // The explicit maintenance path reports what it discarded, and only then can
    // the epoch advance.
    assert_eq!(table.purge(), 1);
    assert!(table.is_empty());
    table.roll_epoch(2).unwrap();
    assert_eq!(table.epoch(), 2);
    // A record from the old epoch is now expired rather than silently gone.
    assert_eq!(table.find(receipt(1, 1).retry), Err(Error::ExpiredEpoch));
    table.retain(receipt(2, 1)).unwrap();
}

#[test]
fn a_receipt_round_trips_and_a_corrupt_one_is_refused() {
    let value = receipt(3, 9);
    let bytes = value.encode();
    assert_eq!(bytes.len(), RECEIPT_BYTES);
    assert_eq!(Receipt6::decode(&bytes), Ok(value));

    let mut flipped = bytes;
    flipped[16] ^= 1;
    assert_eq!(Receipt6::decode(&flipped), Err(Error::Corrupt));
    let mut zero_id = bytes;
    zero_id[32..36].copy_from_slice(&0u32.to_le_bytes());
    let checksum = recompute(&zero_id);
    zero_id[RECEIPT_BYTES - 4..].copy_from_slice(&checksum.to_le_bytes());
    assert_eq!(Receipt6::decode(&zero_id), Err(Error::Corrupt));
    let mut backwards = bytes;
    backwards[44..52].copy_from_slice(&1u64.to_le_bytes());
    let checksum = recompute(&backwards);
    backwards[RECEIPT_BYTES - 4..].copy_from_slice(&checksum.to_le_bytes());
    assert_eq!(Receipt6::decode(&backwards), Err(Error::Corrupt));
    // A zeroed record is an empty slot, not a corrupt one.
    assert_eq!(
        Receipt6::decode(&[0; RECEIPT_BYTES]).err(),
        Some(Error::Corrupt)
    );
}

#[test]
fn a_whole_table_round_trips_and_keeps_its_identity() {
    let mut table = Receipts6::new(LINEAGE).unwrap();
    let epoch = table.epoch();
    for key in 1..=RETAINED_V6 as u64 {
        table.retain(receipt(epoch, key)).unwrap();
    }
    // It still holds evidence, so the epoch cannot move.
    assert_eq!(table.roll_epoch(epoch + 1), Err(Error::Full));
    let bytes = table.encode();
    let restored = Receipts6::decode(LINEAGE, epoch, &bytes).unwrap();
    assert_eq!(restored.len(), RETAINED_V6);
    assert_eq!(restored.checksum(), table.checksum());
    for key in 1..=RETAINED_V6 as u64 {
        assert!(restored.find(receipt(epoch, key).retry).unwrap().is_some());
    }
    // A table from another lineage cannot be adopted.
    let mut foreign = bytes;
    foreign[..16].copy_from_slice(&[9; 16]);
    assert_eq!(
        Receipts6::decode(LINEAGE, epoch, &foreign).err(),
        Some(Error::Corrupt)
    );
    // A corrupt record anywhere in the table fails the whole decode.
    let mut broken = bytes;
    broken[RECEIPT_BYTES * 3 + 40] ^= 1;
    assert_eq!(
        Receipts6::decode(LINEAGE, epoch, &broken).err(),
        Some(Error::Corrupt)
    );
}

fn recompute(bytes: &[u8; RECEIPT_BYTES]) -> u32 {
    let mut value = !0u32;
    for byte in &bytes[..RECEIPT_BYTES - 4] {
        value ^= u32::from(*byte);
        for _ in 0..8 {
            value = (value >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(value & 1)));
        }
    }
    !value
}
