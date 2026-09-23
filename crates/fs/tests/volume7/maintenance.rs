// SPDX-License-Identifier: Apache-2.0
//! Explicit v7 retention and retry-epoch maintenance behavior.

use super::super::*;
use super::{identity, seed_full_file_history, seed_one_file};
use rustic_fs::WriteIdentity7;

fn run_two_replacements(
    disk: &mut Sparse,
) -> (Volume7, format7::Record7, format7::Record7, WriteIdentity7) {
    seed_one_file(disk, b"before");
    let mut volume = Volume7::EMPTY;
    volume.mount_into(disk).unwrap();
    let first = volume
        .replace_tracked(disk, identity(41), 1, b"first!")
        .unwrap();
    let second_identity = identity(42);
    let second = volume
        .replace_tracked(disk, second_identity, first.committed, b"current")
        .unwrap();
    (volume, first, second, second_identity)
}

fn read_live(disk: &mut Sparse, node: &Node7) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(node.length as usize);
    let mut remaining = node.length as usize;
    let mut block = [0u8; 512];
    for run in node.runs() {
        for sector in 0..run.sectors {
            if remaining == 0 {
                return bytes;
            }
            disk.read(format7::PAYLOAD_SECTOR + run.start + sector, &mut block)
                .unwrap();
            let count = remaining.min(block.len());
            bytes.extend_from_slice(&block[..count]);
            remaining -= count;
        }
    }
    assert_eq!(remaining, 0);
    bytes
}

fn allocated(map: &[u64; MAP_WORDS], sector: u64) -> bool {
    map[sector as usize / 64] & (1u64 << (sector % 64)) != 0
}

#[test]
fn full_table_can_be_maintained_then_accepts_a_new_epoch() {
    let mut disk = Sparse::default();
    seed_full_file_history(&mut disk);
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    assert!(
        volume
            .retained_records()
            .unwrap()
            .iter()
            .all(Option::is_some)
    );

    assert_eq!(volume.maintain_retention(&mut disk), Ok(2));
    assert_eq!(volume.header().unwrap().epoch, 2);
    assert_eq!(volume.header().unwrap().sequence, 10);
    assert!(
        volume
            .retained_records()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );

    let new_identity = WriteIdentity7 {
        retry_epoch: 2,
        ..identity(201)
    };
    let record = volume
        .replace_tracked(&mut disk, new_identity, 9, b"new epoch")
        .unwrap();
    assert_eq!(record.retry_epoch, 2);
    assert_eq!(record.committed, 11);
    assert_eq!(volume.node(5).unwrap().unwrap().version, 11);

    let operations = disk.operations;
    assert_eq!(
        volume
            .replace_tracked(&mut disk, identity(100), 1, b"old")
            .err(),
        Some(Error::ExpiredEpoch)
    );
    assert_eq!(disk.operations, operations);
}

#[test]
fn maintenance_prunes_snapshots_but_reclaims_only_sectors_without_live_owners() {
    let mut disk = Sparse::default();
    let (mut volume, first, second, second_identity) = run_two_replacements(&mut disk);
    let before = *volume.header().unwrap();
    let free_before = volume.free_sectors().unwrap();
    let first_snapshot_sector = first.runs()[0].start;
    let live_sector = second.runs()[0].start;
    assert_ne!(first_snapshot_sector, live_sector);
    assert_eq!(volume.free_sectors().unwrap(), rustic_fs::DATA_SECTORS - 2);

    assert_eq!(volume.maintain_retention(&mut disk), Ok(2));
    let header = *volume.header().unwrap();
    assert_eq!(header.lineage, before.lineage);
    assert_eq!(header.next, before.next);
    assert_eq!(header.sequence, before.sequence + 1);
    assert_eq!(header.epoch, before.epoch + 1);
    assert_eq!(volume.free_sectors(), Ok(free_before + 1));
    assert!(
        volume
            .retained_records()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
    let map = volume.allocation_map().unwrap();
    assert!(!allocated(map, first_snapshot_sector));
    assert!(allocated(map, live_sector));

    let live = *volume.node(5).unwrap().unwrap();
    assert_eq!(live.version, second.committed);
    assert_eq!(read_live(&mut disk, &live), b"current");

    let operations = disk.operations;
    assert_eq!(
        volume
            .replace_tracked(&mut disk, second_identity, second.previous, b"current")
            .err(),
        Some(Error::ExpiredEpoch)
    );
    assert_eq!(disk.operations, operations);

    let mut durable = disk.recover();
    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut durable).unwrap();
    assert_eq!(remounted.header().unwrap().lineage, before.lineage);
    assert_eq!(remounted.header().unwrap().next, before.next);
    assert_eq!(remounted.header().unwrap().epoch, 2);
    assert_eq!(remounted.node(5).unwrap().unwrap().id, 5);
    assert_eq!(
        remounted.node(5).unwrap().unwrap().version,
        second.committed
    );
    assert_eq!(remounted.free_sectors(), Ok(free_before + 1));
    assert!(
        remounted
            .retained_records()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
    let remounted_live = *remounted.node(5).unwrap().unwrap();
    assert_eq!(read_live(&mut durable, &remounted_live), b"current");
}

#[test]
fn an_admitted_record_blocks_maintenance_without_writes_or_owner_changes() {
    let mut disk = Sparse::default();
    let (mut nodes, mut records, mut map, mut header) = empty_generation(0, 2);
    header.next = 6;
    nodes[4] = file_node(5, 1, 0, b"live");
    allocate_run(&mut map, 0, 1);
    allocate_run(&mut map, 1, 1);
    write_payload(&mut disk, 0, b"live");
    write_payload(&mut disk, 1, b"pending");
    let mut extents = [Extent::new(0, 0); MAX_EXTENTS];
    extents[0] = Extent::new(1, 1);
    records[0] = Some(Record7 {
        subject: 9,
        workspace: 4,
        object: 5,
        instance: 1,
        retry_epoch: 1,
        retry_key: 99,
        previous: 1,
        committed: 0,
        admission_number: 2,
        terminal: 0,
        length: 7,
        payload_crc32: format7::aggregate(b"pending"),
        state: RecordState::Admitted,
        prevention: None,
        extents_used: 1,
        extents,
    });
    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();

    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    let old_header = *volume.header().unwrap();
    let old_records = *volume.retained_records().unwrap();
    let old_map = *volume.allocation_map().unwrap();
    disk.operations = 0;

    assert_eq!(volume.maintain_retention(&mut disk), Err(Error::Busy));
    assert_eq!(disk.operations, 0);
    assert_eq!(*volume.header().unwrap(), old_header);
    assert_eq!(*volume.retained_records().unwrap(), old_records);
    assert_eq!(*volume.allocation_map().unwrap(), old_map);
}

#[test]
fn terminal_admission_states_are_pruned_without_freeing_disjoint_live_files() {
    let mut disk = Sparse::default();
    let (mut nodes, mut records, mut map, mut header) = empty_generation(0, 5);
    header.next = 8;
    nodes[4] = file_node(5, 5, 1, b"current");
    let other = [0x35; 600];
    nodes[5] = file_node(6, 1, 10, &other);
    nodes[5].extents_used = 2;
    nodes[5].extents[0] = Extent::new(10, 1);
    nodes[5].extents[1] = Extent::new(12, 1);
    nodes[5].name = name_field(b"other");
    nodes[5].name_length = 5;
    allocate_run(&mut map, 0, 1);
    allocate_run(&mut map, 1, 1);
    allocate_run(&mut map, 10, 1);
    allocate_run(&mut map, 12, 1);
    write_payload(&mut disk, 0, b"aborted");
    write_payload(&mut disk, 1, b"current");
    disk.write(
        format7::PAYLOAD_SECTOR + 10,
        &other[..512].try_into().unwrap(),
    )
    .unwrap();
    let mut other_tail = [0; 512];
    other_tail[..other.len() - 512].copy_from_slice(&other[512..]);
    disk.write(format7::PAYLOAD_SECTOR + 12, &other_tail)
        .unwrap();

    let mut cancelled_extents = [Extent::new(0, 0); MAX_EXTENTS];
    cancelled_extents[0] = Extent::new(0, 1);
    records[0] = Some(Record7 {
        subject: 9,
        workspace: 4,
        object: 7,
        instance: 1,
        retry_epoch: 1,
        retry_key: 71,
        previous: 1,
        committed: 0,
        admission_number: 2,
        terminal: 3,
        length: 7,
        payload_crc32: format7::aggregate(b"aborted"),
        state: RecordState::Cancelled,
        prevention: Some(rustic_fs::PreventionReason::Requested),
        extents_used: 1,
        extents: cancelled_extents,
    });
    let mut committed_extents = [Extent::new(0, 0); MAX_EXTENTS];
    committed_extents[0] = Extent::new(1, 1);
    records[1] = Some(Record7 {
        subject: 9,
        workspace: 4,
        object: 5,
        instance: 1,
        retry_epoch: 1,
        retry_key: 72,
        previous: 1,
        committed: 5,
        admission_number: 4,
        terminal: 5,
        length: 7,
        payload_crc32: format7::aggregate(b"current"),
        state: RecordState::AdmittedCommitted,
        prevention: None,
        extents_used: 1,
        extents: committed_extents,
    });
    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();

    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    assert_eq!(volume.free_sectors(), Ok(rustic_fs::DATA_SECTORS - 4));
    assert_eq!(volume.maintain_retention(&mut disk), Ok(2));
    assert_eq!(volume.free_sectors(), Ok(rustic_fs::DATA_SECTORS - 3));
    assert!(
        volume
            .retained_records()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );

    let map = volume.allocation_map().unwrap();
    assert!(!allocated(map, 0));
    assert!(allocated(map, 1));
    assert!(allocated(map, 10));
    assert!(allocated(map, 12));
    for id in [5, 6] {
        let node = *volume.node(id).unwrap().unwrap();
        let expected = if id == 5 {
            b"current".as_slice()
        } else {
            other.as_slice()
        };
        assert_eq!(read_live(&mut disk, &node), expected);
    }

    let mut durable = disk.recover();
    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut durable).unwrap();
    assert_eq!(remounted.free_sectors(), Ok(rustic_fs::DATA_SECTORS - 3));
    for id in [5, 6] {
        let node = *remounted.node(id).unwrap().unwrap();
        let expected = if id == 5 {
            b"current".as_slice()
        } else {
            other.as_slice()
        };
        assert_eq!(read_live(&mut durable, &node), expected);
    }
}

#[test]
fn epoch_and_sequence_overflow_refuse_before_writing() {
    for epoch in [u64::MAX, 2] {
        let mut disk = Sparse::default();
        let (nodes, records, map, mut header) = empty_generation(0, u64::MAX);
        header.epoch = epoch;
        persist_generation(&mut disk, header, &nodes, &records, &map);
        disk.flush().unwrap();

        let mut volume = Volume7::EMPTY;
        volume.mount_into(&mut disk).unwrap();
        let old_header = *volume.header().unwrap();
        disk.operations = 0;

        assert_eq!(volume.maintain_retention(&mut disk), Err(Error::Exhausted));
        assert_eq!(disk.operations, 0);
        assert_eq!(*volume.header().unwrap(), old_header);
    }
}

#[test]
fn every_maintenance_write_and_flush_cut_remounts_the_old_generation() {
    const PUBLICATION_STEPS: usize = 103;

    for fail_at in 0..PUBLICATION_STEPS {
        let mut disk = Sparse::default();
        seed_full_file_history(&mut disk);
        let mut volume = Volume7::EMPTY;
        volume.mount_into(&mut disk).unwrap();
        let old_header = *volume.header().unwrap();
        let old_records = *volume.retained_records().unwrap();
        disk.operations = 0;
        disk.fail_at = Some(fail_at);

        assert_eq!(
            volume.maintain_retention(&mut disk),
            Err(Error::Uncertain),
            "publication step {fail_at} must be uncertain"
        );
        assert_eq!(volume.header().err(), Some(Error::Uncertain));

        let mut durable = disk.recover();
        durable.fail_at = None;
        let mut recovered = Volume7::EMPTY;
        recovered.mount_into(&mut durable).unwrap();
        assert_eq!(
            *recovered.header().unwrap(),
            old_header,
            "durable recovery at step {fail_at} must select the complete old head"
        );
        assert_eq!(*recovered.retained_records().unwrap(), old_records);
    }
}

struct DurableFlushFailure<'a> {
    disk: &'a mut Sparse,
    flushes: usize,
    fail_at: usize,
}

impl Disk for DurableFlushFailure<'_> {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        self.disk.read(sector, bytes)
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        self.disk.write(sector, bytes)
    }

    fn flush(&mut self) -> Result<(), Error> {
        let current = self.flushes;
        self.flushes += 1;
        self.disk.flush()?;
        if current == self.fail_at {
            Err(Error::Io)
        } else {
            Ok(())
        }
    }
}

#[test]
fn a_final_flush_error_after_durable_maintenance_remounts_the_new_head() {
    let mut disk = Sparse::default();
    seed_full_file_history(&mut disk);
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();

    assert_eq!(
        volume.maintain_retention(&mut DurableFlushFailure {
            disk: &mut disk,
            flushes: 0,
            fail_at: 1,
        }),
        Err(Error::Uncertain)
    );
    assert_eq!(volume.header().err(), Some(Error::Uncertain));

    volume.mount_into(&mut disk).unwrap();
    assert_eq!(volume.header().unwrap().sequence, 10);
    assert_eq!(volume.header().unwrap().epoch, 2);
    assert!(
        volume
            .retained_records()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
}
