// SPDX-License-Identifier: Apache-2.0
//! Host tests for v6 tracked writes: data, version and operation identity
//! published by one commit (#51).
mod support;

use rustic_fs::{DATA_SECTORS, Error, Kind, Node6, RETAINED_V6, Retry, mount6, provision6};
use support::Sparse;

const LINEAGE: [u8; 16] = [7; 16];

fn retry(key: u64) -> Retry {
    Retry {
        lineage: LINEAGE,
        epoch: 1,
        key,
    }
}

/// Leave a file record at `slot` whose identity is `id` and version is `expected`.
fn file(volume: &mut rustic_fs::Volume6, slot: usize, id: u32, expected: u64) {
    let mut record = Node6::EMPTY;
    record.id = id;
    record.parent = 4;
    record.kind = Kind::File;
    record.version = expected;
    record.name[..4].copy_from_slice(b"file");
    record.name_length = 4;
    volume.nodes[slot] = record;
}

#[test]
fn a_tracked_write_publishes_data_version_and_receipt_together() {
    let mut disk = Sparse::default();
    let mut volume = provision6(&mut disk, LINEAGE).expect("provision");
    file(&mut volume, 4, 5, 1);
    let receipt = volume
        .write_tracked(&mut disk, 4, 1, retry(11), b"payload v1")
        .expect("tracked write");
    assert_eq!(
        (
            receipt.id,
            receipt.previous,
            receipt.committed,
            receipt.length
        ),
        (5, 1, 2, 10)
    );

    // One commit published the bytes, the version and the evidence of them.
    let mut disk = disk.recover();
    let mounted = mount6(&mut disk).expect("mount");
    let node = mounted.node(5).expect("file node");
    assert_eq!(node.version, receipt.committed);
    assert_eq!(node.length as usize, receipt.length as usize);
    assert_eq!(node.runs().iter().map(|run| run.sectors).sum::<u64>(), 1);
    let mut bytes = vec![0; node.length as usize];
    assert_eq!(
        mounted.read_file(&mut disk, node, &mut bytes),
        Ok(bytes.len())
    );
    assert_eq!(bytes, b"payload v1");
    assert_eq!(mounted.find_receipt(retry(11)), Ok(Some(&receipt)));
    assert_eq!(mounted.free_sectors(), DATA_SECTORS - 1);
}

#[test]
fn replaying_an_identical_retry_writes_nothing() {
    let mut disk = Sparse::default();
    let mut volume = provision6(&mut disk, LINEAGE).expect("provision");
    file(&mut volume, 4, 5, 1);
    let receipt = volume
        .write_tracked(&mut disk, 4, 1, retry(11), b"payload v1")
        .expect("tracked write");
    let operations = disk.operations;

    // A resubmission after an unknown outcome repeats the identity, not the work.
    assert_eq!(
        volume.write_tracked(&mut disk, 4, 1, retry(11), b"payload v1"),
        Ok(receipt)
    );
    assert_eq!(disk.operations, operations);
    assert_eq!(volume.nodes[4].version, receipt.committed);
    assert_eq!(volume.free_sectors(), DATA_SECTORS - 1);
}

#[test]
fn a_retry_that_describes_another_record_is_a_conflict_or_expiry() {
    let mut disk = Sparse::default();
    let mut volume = provision6(&mut disk, LINEAGE).expect("provision");
    file(&mut volume, 4, 5, 1);
    volume
        .write_tracked(&mut disk, 4, 1, retry(11), b"payload v1")
        .expect("tracked write");
    let operations = disk.operations;

    // The same key with another length, or another expected version, is not the
    // retained operation.
    assert_eq!(
        volume.write_tracked(&mut disk, 4, 1, retry(11), b"other"),
        Err(Error::IdempotencyConflict)
    );
    assert_eq!(
        volume.write_tracked(&mut disk, 4, 2, retry(11), b"payload v1"),
        Err(Error::IdempotencyConflict)
    );
    // An unknown key in an epoch the table has left is expired, not a new
    // operation; the table's own epoch is still 1, so key 12 there is new.
    assert_eq!(
        volume.write_tracked(
            &mut disk,
            4,
            2,
            Retry {
                lineage: LINEAGE,
                epoch: 2,
                key: 12
            },
            b"payload v2"
        ),
        Err(Error::ExpiredEpoch)
    );
    assert_eq!(
        volume.write_tracked(
            &mut disk,
            4,
            2,
            Retry {
                lineage: [9; 16],
                epoch: 1,
                key: 12
            },
            b"payload v2"
        ),
        Err(Error::Lineage)
    );
    assert_eq!(disk.operations, operations);
    assert_eq!(volume.nodes[4].version, 2);
}

#[test]
fn a_full_table_is_refused_before_any_payload_is_staged() {
    let mut disk = Sparse::default();
    let mut volume = provision6(&mut disk, LINEAGE).expect("provision");
    for slot in 4..4 + RETAINED_V6 {
        file(&mut volume, slot, slot as u32 + 1, 1);
        volume
            .write_tracked(&mut disk, slot, 1, retry(slot as u64), b"held")
            .expect("tracked write");
    }
    assert_eq!(volume.receipts.len(), RETAINED_V6);
    let held = DATA_SECTORS - volume.free_sectors();
    let operations = disk.operations;

    // The ninth identity is refused honestly: no eviction, no payload, no commit.
    file(&mut volume, 4 + RETAINED_V6, 20, 1);
    assert_eq!(
        volume.write_tracked(&mut disk, 4 + RETAINED_V6, 1, retry(99), b"too late"),
        Err(Error::Full)
    );
    assert_eq!(disk.operations, operations);
    assert_eq!(volume.nodes[4 + RETAINED_V6].version, 1);
    assert_eq!(DATA_SECTORS - volume.free_sectors(), held);
}

#[test]
fn a_failed_commit_publishes_neither_the_data_nor_the_receipt() {
    let mut disk = Sparse::default();
    let mut volume = provision6(&mut disk, LINEAGE).expect("provision");
    file(&mut volume, 4, 5, 1);
    volume
        .write_file(&mut disk, 4, 1, b"committed first")
        .expect("first write");
    let mut disk = disk.recover();

    disk.fail_at = Some(0);
    let mut volume = mount6(&mut disk).expect("mount");
    assert_eq!(
        volume.write_tracked(&mut disk, 4, 2, retry(11), b"never published"),
        Err(Error::Io)
    );
    disk.fail_at = None;
    let mut disk = disk.recover();
    let mounted = mount6(&mut disk).expect("remount");
    let node = mounted.node(5).expect("file node");
    assert_eq!(node.version, 2);
    assert_eq!(mounted.find_receipt(retry(11)), Ok(None));
    let mut bytes = vec![0; node.length as usize];
    assert_eq!(
        mounted.read_file(&mut disk, node, &mut bytes),
        Ok(bytes.len())
    );
    assert_eq!(bytes, b"committed first");
}
