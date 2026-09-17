// SPDX-License-Identifier: Apache-2.0
//! Host tests for the v6 volume layer: provision, mount, verified structures and
//! payload I/O through extents (#51).
mod support;

use rustic_fs::{
    DATA_BYTES_V6, Disk, Error, Kind, MAX_FILE_V6, Node6, Receipt6, Retry, mount6, provision6,
};
use support::Sparse;

const LINEAGE: [u8; 16] = [7; 16];

#[test]
fn provision_then_mount_restores_the_roots_and_the_whole_free_region() {
    let mut disk = Sparse::default();
    // An unformatted disk is not mountable: the header is missing, not guessed.
    assert_eq!(mount6(&mut disk).err(), Some(Error::Corrupt));

    let volume = provision6(&mut disk, LINEAGE).expect("provision");
    assert_eq!(volume.free_sectors(), rustic_fs::DATA_SECTORS);
    let mounted = mount6(&mut disk).expect("mount");
    for (index, name) in ["system", "data", "config", "workspaces"]
        .iter()
        .enumerate()
    {
        let node = mounted.node(index as u32 + 1).expect("root directory");
        assert_eq!(node.kind, Kind::Directory);
        let len = node.name_length as usize;
        assert_eq!(&node.name[..len], name.as_bytes());
    }
    assert_eq!(mounted.free_sectors(), rustic_fs::DATA_SECTORS);
}

#[test]
fn a_large_file_round_trips_across_extents_and_reclaims_exactly() {
    let mut disk = Sparse::default();
    let mut volume = provision6(&mut disk, LINEAGE).expect("provision");
    let initial = volume.free_sectors();

    // The layer owns payload, not identity: the caller creates the record.
    let mut record = Node6::EMPTY;
    record.id = 5;
    record.parent = 4;
    record.kind = Kind::File;
    record.name[..3].copy_from_slice(b"app");
    record.name_length = 3;
    record.version = 1;
    volume.nodes[4] = record;

    // 220,000 bytes: far beyond the v5 record's 16-bit length and one bank.
    let bytes: Vec<u8> = (0..220_000u32).map(|index| (index % 251) as u8).collect();
    assert_eq!(volume.write_file(&mut disk, 4, 1, &bytes), Ok(2));
    let sectors = (bytes.len() as u64).div_ceil(512);
    assert_eq!(volume.free_sectors(), initial - sectors);

    let mounted = mount6(&mut disk).expect("remount");
    let node = mounted.node(5).expect("file node");
    assert_eq!(node.length as usize, bytes.len());
    assert!(node.extents_used >= 1);
    let mut out = vec![0u8; bytes.len()];
    let mut disk = disk.recover();
    assert_eq!(
        mounted.read_file(&mut disk, node, &mut out),
        Ok(bytes.len())
    );
    assert_eq!(out, bytes);

    // Removing the file returns every sector it held.
    let mut volume = mount6(&mut disk).expect("mount for removal");
    volume.remove_file(&mut disk, 4).expect("remove");
    assert_eq!(volume.free_sectors(), initial);
    assert!(mount6(&mut disk).expect("mount").node(5).is_none());
}

#[test]
fn a_file_larger_than_the_limit_is_refused_before_anything_is_written() {
    let mut disk = Sparse::default();
    let mut volume = provision6(&mut disk, LINEAGE).expect("provision");
    let initial = volume.free_sectors();
    let oversized = vec![0u8; MAX_FILE_V6 + 1];
    assert_eq!(
        volume.write_file(&mut disk, 4, 1, &oversized),
        Err(Error::Size)
    );
    assert_eq!(volume.free_sectors(), initial);
    assert!(volume.node(5).is_none());
}

#[test]
fn a_corrupt_or_torn_volume_is_refused_instead_of_mounted() {
    let mut disk = Sparse::default();
    provision6(&mut disk, LINEAGE).expect("provision");
    let mut volume = provision6(&mut disk, LINEAGE).expect("reprovision");
    let mut record = Node6::EMPTY;
    record.id = 5;
    record.parent = 4;
    record.kind = Kind::File;
    record.name[..3].copy_from_slice(b"app");
    record.name_length = 3;
    record.version = 1;
    volume.nodes[4] = record;
    volume
        .write_file(&mut disk, 4, 1, b"payload")
        .expect("write");
    // Structures live in the active generation, which the header names.
    let active = mount6(&mut disk).expect("mount").header.active;
    let nodes_base = rustic_fs::nodes_sector(active);
    let map_base = rustic_fs::map_sector(active);

    // A single flipped byte in the node table must fail the header's checksum.
    let mut torn = disk.recover();
    torn.corrupt(nodes_base, 0);
    assert_eq!(mount6(&mut torn).err(), Some(Error::Corrupt));

    // So must a flipped byte in the free-space map.
    let mut map_torn = disk.recover();
    map_torn.corrupt(map_base, 3);
    assert_eq!(mount6(&mut map_torn).err(), Some(Error::Corrupt));

    // And so must a flipped byte in the header itself.
    let mut header_torn = disk.recover();
    header_torn.corrupt(8, 12);
    assert_eq!(mount6(&mut header_torn).err(), Some(Error::Corrupt));

    // The untorn volume still mounts, so the refusals above are specific.
    assert!(mount6(&mut disk).is_ok());
    assert_eq!(DATA_BYTES_V6, 64 * 1024 * 1024);
    let _ = Node6::EMPTY;
}

#[test]
fn a_torn_commit_cannot_publish_the_inactive_generation() {
    let mut disk = Sparse::default();
    let volume = provision6(&mut disk, LINEAGE).expect("provision");
    let mounted = mount6(&mut disk).expect("mount");
    let active = mounted.header.active;
    let inactive = (active + 1) % rustic_fs::GENERATIONS;
    // Simulate a commit that wrote its structures but never flipped the header:
    // the inactive generation is full of arbitrary bytes.
    for sector in rustic_fs::nodes_sector(inactive)..rustic_fs::nodes_sector(inactive) + 8 {
        disk.write(sector, &[0xa5; 512]).unwrap();
    }
    disk.flush().unwrap();

    // The volume still mounts, and it is the old generation that answers.
    let remounted = mount6(&mut disk).expect("torn commit must not be published");
    assert_eq!(remounted.header.active, active);
    assert!(remounted.node(1).is_some());

    // A commit that does flip the header publishes the new generation, and the
    // previous one is left intact as the fallback.
    let mut volume2 = mount6(&mut disk).expect("mount");
    let mut record = Node6::EMPTY;
    record.id = 9;
    record.parent = 4;
    record.kind = Kind::File;
    record.version = 1;
    volume2.nodes[5] = record;
    volume2
        .write_file(&mut disk, 5, 1, b"committed")
        .expect("commit");
    let published = mount6(&mut disk).expect("mount after commit");
    assert_ne!(published.header.active, active);
    assert!(published.node(9).is_some());
    assert_eq!(volume.free_sectors(), rustic_fs::DATA_SECTORS);
    let _ = inactive;
}

#[test]
fn a_stale_writer_is_refused_and_changes_nothing() {
    let mut disk = Sparse::default();
    let mut volume = provision6(&mut disk, LINEAGE).expect("provision");
    let mut record = Node6::EMPTY;
    record.id = 5;
    record.parent = 4;
    record.kind = Kind::File;
    record.version = 7;
    volume.nodes[4] = record;
    let before = mount6(&mut disk).expect("mount").free_sectors();

    // Right version: accepted, and the record's version advances.
    assert_eq!(volume.write_file(&mut disk, 4, 7, b"first"), Ok(8));
    // Same version again: a stale writer is refused, not silently allowed.
    assert_eq!(
        volume.write_file(&mut disk, 4, 7, b"second"),
        Err(Error::Version)
    );
    // A missing record is not a version conflict.
    assert_eq!(
        volume.write_file(&mut disk, 9, 0, b"third"),
        Err(Error::NotFound)
    );

    let mounted = mount6(&mut disk).expect("mount");
    let node = mounted.node(5).expect("file node");
    assert_eq!(node.version, 8);
    assert_eq!(node.length, 5);
    let mut out = [0u8; 5];
    let mut disk = disk.recover();
    assert_eq!(mounted.read_file(&mut disk, node, &mut out), Ok(5));
    assert_eq!(&out, b"first");
    assert!(mounted.free_sectors() < before);
}

#[test]
fn receipts_are_published_with_the_commit_and_survive_a_remount() {
    let mut disk = Sparse::default();
    let mut volume = provision6(&mut disk, LINEAGE).expect("provision");
    assert_eq!(volume.receipts.len(), 0);
    let receipt = Receipt6 {
        retry: Retry {
            lineage: LINEAGE,
            epoch: volume.receipts.epoch(),
            key: 42,
        },
        id: 5,
        previous: 3,
        committed: 4,
        length: 220_000,
    };
    volume.retain_receipt(&mut disk, receipt).expect("retain");

    // The commit published it: a remount reads it back from the generation.
    let remounted = mount6(&mut disk).expect("remount");
    assert_eq!(remounted.receipts.len(), 1);
    assert_eq!(
        remounted.find_receipt(receipt.retry).unwrap(),
        Some(&receipt)
    );
    // It is bound to this volume's lineage.
    let mut foreign = receipt;
    foreign.retry.lineage = [9; 16];
    assert_eq!(remounted.find_receipt(foreign.retry), Err(Error::Lineage));

    // A corrupted receipt sector fails the mount through the header checksum.
    let mut torn = disk.recover();
    let active = mount6(&mut torn).expect("mount").header.active;
    torn.corrupt(rustic_fs::receipts_sector(active), 4);
    assert_eq!(mount6(&mut torn).err(), Some(Error::Corrupt));
}

#[test]
fn rewriting_a_file_reclaims_the_payload_it_replaced() {
    let mut disk = Sparse::default();
    let mut volume = provision6(&mut disk, LINEAGE).expect("provision");
    let initial = volume.free_sectors();
    let mut record = Node6::EMPTY;
    record.id = 5;
    record.parent = 4;
    record.kind = Kind::File;
    record.version = 1;
    volume.nodes[4] = record;

    let large = vec![7u8; 100_000];
    assert_eq!(volume.write_file(&mut disk, 4, 1, &large), Ok(2));
    assert_eq!(initial - volume.free_sectors(), 100_000u64.div_ceil(512));

    // A rewrite holds only what the new bytes need: the replaced runs return to
    // the map, or repeated writes of one file exhaust the region.
    let small = vec![9u8; 100];
    assert_eq!(volume.write_file(&mut disk, 4, 2, &small), Ok(3));
    assert_eq!(initial - volume.free_sectors(), 1);
    let mounted = mount6(&mut disk).expect("mount");
    let node = mounted.node(5).expect("file node");
    assert_eq!(node.length, 100);
    let mut out = vec![0u8; 100];
    let mut disk = disk.recover();
    assert_eq!(mounted.read_file(&mut disk, node, &mut out), Ok(100));
    assert_eq!(out, small);

    // Truncating to nothing returns everything the file held.
    let mut volume = mount6(&mut disk).expect("mount for truncation");
    assert_eq!(volume.write_file(&mut disk, 4, 3, &[]), Ok(4));
    assert_eq!(volume.free_sectors(), initial);
}

#[test]
fn a_range_read_streams_only_the_extents_a_range_needs() {
    let mut disk = Sparse::default();
    let mut volume = provision6(&mut disk, LINEAGE).expect("provision");
    let mut record = Node6::EMPTY;
    record.id = 5;
    record.parent = 4;
    record.kind = Kind::File;
    record.version = 1;
    volume.nodes[4] = record;
    let bytes: Vec<u8> = (0..200_000u32).map(|index| (index % 251) as u8).collect();
    volume.write_file(&mut disk, 4, 1, &bytes).expect("write");
    let mounted = mount6(&mut disk).expect("mount");
    let node = mounted.node(5).expect("file node");
    assert!(node.runs().len() > 1, "the fixture must span runs");

    // A whole-file read and a range read of the whole file agree.
    let mut all = vec![0; bytes.len()];
    assert_eq!(
        mounted.read_file(&mut disk, node, &mut all),
        Ok(bytes.len())
    );
    assert_eq!(all, bytes);
    let mut whole = vec![0; bytes.len()];
    assert_eq!(
        mounted.read_range(&mut disk, node, 0, &mut whole),
        Ok(bytes.len())
    );
    assert_eq!(whole, bytes);

    // A range that starts and ends inside different runs, and one that crosses
    // several, both return exactly the bytes at those offsets.
    for (offset, count) in [(0usize, 1usize), (511, 2), (100_000, 4096)] {
        let mut part = vec![0; count];
        assert_eq!(
            mounted.read_range(&mut disk, node, offset as u64, &mut part),
            Ok(count)
        );
        assert_eq!(part, bytes[offset..offset + count]);
    }
    // A range that asks for more than the file holds stops at the end.
    let mut over = vec![0; 10_000];
    assert_eq!(
        mounted.read_range(&mut disk, node, 199_000, &mut over),
        Ok(1_000)
    );
    assert_eq!(&over[..1_000], &bytes[199_000..]);
    // A range past the end copies the remainder, and one past the length is refused.
    let mut tail = vec![0; 100];
    assert_eq!(
        mounted.read_range(&mut disk, node, 199_999, &mut tail),
        Ok(1)
    );
    assert_eq!(tail[0], bytes[199_999]);
    assert_eq!(
        mounted.read_range(&mut disk, node, bytes.len() as u64, &mut tail),
        Ok(0)
    );
    assert_eq!(
        mounted.read_range(&mut disk, node, bytes.len() as u64 + 1, &mut tail),
        Err(Error::Size)
    );

    // A record whose extents cannot hold its length is refused, not read short.
    let mut lying = *node;
    lying.length = 400_000;
    assert_eq!(
        mounted.read_range(&mut disk, &lying, 0, &mut tail),
        Err(Error::Corrupt)
    );
    assert_eq!(
        mounted.read_file(&mut disk, &lying, &mut vec![0; 400_000]),
        Err(Error::Corrupt)
    );
    assert_eq!(
        mounted.read_file(&mut disk, node, &mut tail),
        Err(Error::Size)
    );
}
