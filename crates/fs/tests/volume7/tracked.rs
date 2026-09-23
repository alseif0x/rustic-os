// SPDX-License-Identifier: Apache-2.0
//! Tracked direct replacement and dual-generation publication for v7.

use super::*;
use rustic_fs::WriteIdentity7;

fn identity(key: u64) -> WriteIdentity7 {
    WriteIdentity7 {
        subject: 9,
        workspace: 4,
        object: 5,
        instance: 1,
        retry_epoch: 1,
        retry_key: key,
    }
}

fn named_file(id: u32, version: u64, start: u64, data: &[u8], name: &[u8]) -> Node7 {
    let mut node = file_node(id, version, start, data);
    node.name = name_field(name);
    node.name_length = name.len() as u8;
    node
}

fn seed_one_file(disk: &mut Sparse, initial: &[u8]) {
    let (mut nodes, records, mut map, mut header) = empty_generation(0, 1);
    header.next = 6;
    nodes[4] = file_node(5, 1, 0, initial);
    allocate_run(&mut map, 0, 1);
    write_payload(disk, 0, initial);
    persist_generation(disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
}

fn seed_full_file_history(disk: &mut Sparse) {
    let (mut nodes, mut records, map, mut header) = empty_generation(0, 9);
    header.next = 6;
    nodes[4] = named_file(5, 9, 0, b"", b"live");
    nodes[4].extents_used = 0;
    nodes[4].extents = [Extent::new(0, 0); MAX_EXTENTS];
    for (index, slot) in records.iter_mut().enumerate() {
        let previous = index as u64 + 1;
        let committed = previous + 1;
        *slot = Some(Record7 {
            subject: 9,
            workspace: 4,
            object: 5,
            instance: 1,
            retry_epoch: 1,
            retry_key: 100 + index as u64,
            previous,
            committed,
            admission_number: 0,
            terminal: committed,
            length: 0,
            payload_crc32: format7::aggregate(&[]),
            state: RecordState::DirectCommitted,
            prevention: None,
            extents_used: 0,
            extents: [Extent::new(0, 0); MAX_EXTENTS],
        });
    }
    persist_generation(disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
}

fn seed_missing_file(disk: &mut Sparse) {
    let (nodes, records, map, mut header) = empty_generation(0, 1);
    header.next = 6;
    persist_generation(disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
}

fn seed_exhausted_sequence(disk: &mut Sparse) {
    let (mut nodes, records, mut map, mut header) = empty_generation(0, u64::MAX);
    header.next = 6;
    nodes[4] = file_node(5, 1, 0, b"before");
    allocate_run(&mut map, 0, 1);
    write_payload(disk, 0, b"before");
    persist_generation(disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
}

fn read_snapshot(disk: &mut Sparse, record: &Record7) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(record.length as usize);
    let mut block = [0u8; 512];
    let mut remaining = record.length as usize;
    for run in record.runs() {
        for offset in 0..run.sectors {
            if remaining == 0 {
                return bytes;
            }
            disk.read(format7::PAYLOAD_SECTOR + run.start + offset, &mut block)
                .unwrap();
            let count = remaining.min(block.len());
            bytes.extend_from_slice(&block[..count]);
            remaining -= count;
        }
    }
    assert_eq!(remaining, 0);
    bytes
}

#[test]
fn tracked_replacement_publishes_and_remounts_durably() {
    let mut disk = Sparse::default();
    seed_one_file(&mut disk, b"before");
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();

    let record = volume
        .replace_tracked(&mut disk, identity(11), 1, b"after!")
        .unwrap();
    assert_eq!(record.state, RecordState::DirectCommitted);
    assert_eq!(record.previous, 1);
    assert_eq!(record.committed, 2);
    assert_eq!(record.terminal, 2);
    assert_eq!(volume.node(5).unwrap().unwrap().version, 2);
    assert_eq!(volume.retained_records().unwrap()[0], Some(record));
    assert_eq!(read_snapshot(&mut disk, &record), b"after!");

    let mut durable = disk.recover();
    durable.fail_at = None;
    let mut mounted = Volume7::EMPTY;
    mounted.mount_into(&mut durable).unwrap();
    assert_eq!(mounted.header().unwrap().sequence, 2);
    assert_eq!(mounted.node(5).unwrap().unwrap().version, 2);
    assert_eq!(mounted.retained_records().unwrap()[0], Some(record));
    assert_eq!(read_snapshot(&mut durable, &record), b"after!");
}

#[test]
fn exact_retry_reads_snapshot_but_performs_no_write_or_flush() {
    let mut disk = Sparse::default();
    seed_one_file(&mut disk, b"before");
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    let durable = volume
        .replace_tracked(&mut disk, identity(12), 1, b"after!")
        .unwrap();
    let operations = disk.operations;

    let replay = volume
        .replace_tracked(&mut disk, identity(12), 1, b"after!")
        .unwrap();
    assert_eq!(replay, durable);
    assert_eq!(disk.operations, operations);
    assert_eq!(volume.header().unwrap().sequence, 2);
}

#[test]
fn corrupt_retry_snapshot_is_detected_without_fencing_or_writing() {
    let mut disk = Sparse::default();
    seed_one_file(&mut disk, b"before");
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    let record = volume
        .replace_tracked(&mut disk, identity(22), 1, b"after!")
        .unwrap();
    disk.corrupt(format7::PAYLOAD_SECTOR + record.runs()[0].start, 0);
    let operations = disk.operations;

    assert_eq!(
        volume
            .replace_tracked(&mut disk, identity(22), 1, b"after!")
            .err(),
        Some(Error::Corrupt)
    );
    assert_eq!(disk.operations, operations);
    assert_eq!(volume.header().unwrap().sequence, 2);
}

struct RetryReadFailure<'a>(&'a mut Sparse);

impl Disk for RetryReadFailure<'_> {
    fn read(&mut self, _: u64, _: &mut [u8; 512]) -> Result<(), Error> {
        Err(Error::Io)
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        self.0.write(sector, bytes)
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.0.flush()
    }
}

#[test]
fn retry_read_failure_does_not_fence_or_write() {
    let mut disk = Sparse::default();
    seed_one_file(&mut disk, b"before");
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    volume
        .replace_tracked(&mut disk, identity(23), 1, b"after!")
        .unwrap();
    let operations = disk.operations;

    assert_eq!(
        volume
            .replace_tracked(&mut RetryReadFailure(&mut disk), identity(23), 1, b"after!")
            .err(),
        Some(Error::Io)
    );
    assert_eq!(disk.operations, operations);
    assert_eq!(volume.header().unwrap().sequence, 2);
}

#[test]
fn retry_identity_reuse_with_different_bytes_or_metadata_conflicts() {
    let mut disk = Sparse::default();
    seed_one_file(&mut disk, b"before");
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    volume
        .replace_tracked(&mut disk, identity(13), 1, b"after!")
        .unwrap();
    let operations = disk.operations;

    assert_eq!(
        volume
            .replace_tracked(&mut disk, identity(13), 1, b"alter!")
            .err(),
        Some(Error::IdempotencyConflict)
    );
    assert_eq!(
        volume
            .replace_tracked(&mut disk, identity(13), 2, b"after!")
            .err(),
        Some(Error::IdempotencyConflict)
    );
    assert_eq!(disk.operations, operations);
    assert_eq!(volume.header().unwrap().sequence, 2);
}

#[test]
fn stale_version_and_full_retention_refuse_before_any_write_or_flush() {
    let mut disk = Sparse::default();
    seed_one_file(&mut disk, b"before");
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    disk.operations = 0;
    assert_eq!(
        volume
            .replace_tracked(&mut disk, identity(14), 2, b"after!")
            .err(),
        Some(Error::Version)
    );
    assert_eq!(disk.operations, 0);
    assert_eq!(volume.header().unwrap().sequence, 1);

    let mut full_disk = Sparse::default();
    seed_full_file_history(&mut full_disk);
    let mut full = Volume7::EMPTY;
    full.mount_into(&mut full_disk).unwrap();
    full_disk.operations = 0;
    assert_eq!(
        full.replace_tracked(&mut full_disk, identity(15), 9, b"never")
            .err(),
        Some(Error::Full)
    );
    assert_eq!(full_disk.operations, 0);
    assert_eq!(full.header().unwrap().sequence, 9);
}

#[test]
fn size_identity_epoch_not_found_and_sequence_refusals_do_not_write() {
    let mut disk = Sparse::default();
    seed_one_file(&mut disk, b"before");
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    disk.operations = 0;

    assert_eq!(
        volume
            .replace_tracked(
                &mut disk,
                WriteIdentity7 {
                    subject: 0,
                    ..identity(25)
                },
                1,
                b"after!",
            )
            .err(),
        Some(Error::Invalid)
    );
    assert_eq!(
        volume
            .replace_tracked(
                &mut disk,
                WriteIdentity7 {
                    retry_epoch: 2,
                    ..identity(26)
                },
                1,
                b"after!",
            )
            .err(),
        Some(Error::ExpiredEpoch)
    );
    assert_eq!(
        volume
            .replace_tracked(&mut disk, identity(27), 1, &vec![0; 256 * 1024 + 1])
            .err(),
        Some(Error::Size)
    );
    assert_eq!(disk.operations, 0);
    assert_eq!(volume.header().unwrap().sequence, 1);

    let mut missing_disk = Sparse::default();
    seed_missing_file(&mut missing_disk);
    let mut missing = Volume7::EMPTY;
    missing.mount_into(&mut missing_disk).unwrap();
    missing_disk.operations = 0;
    assert_eq!(
        missing
            .replace_tracked(&mut missing_disk, identity(28), 1, b"after!")
            .err(),
        Some(Error::NotFound)
    );
    assert_eq!(missing_disk.operations, 0);

    let mut exhausted_disk = Sparse::default();
    seed_exhausted_sequence(&mut exhausted_disk);
    let mut exhausted = Volume7::EMPTY;
    exhausted.mount_into(&mut exhausted_disk).unwrap();
    exhausted_disk.operations = 0;
    assert_eq!(
        exhausted
            .replace_tracked(&mut exhausted_disk, identity(29), 1, b"after!")
            .err(),
        Some(Error::Exhausted)
    );
    assert_eq!(exhausted_disk.operations, 0);
    assert_eq!(exhausted.header().unwrap().sequence, u64::MAX);
}

#[test]
fn a_later_replacement_keeps_the_previous_retained_snapshot() {
    let mut disk = Sparse::default();
    seed_one_file(&mut disk, b"before");
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    let first = volume
        .replace_tracked(&mut disk, identity(16), 1, b"first!")
        .unwrap();
    let second_identity = WriteIdentity7 {
        retry_key: 17,
        ..identity(17)
    };
    let second = volume
        .replace_tracked(&mut disk, second_identity, first.committed, b"second")
        .unwrap();

    assert_ne!(first.extents, second.extents);
    assert_eq!(read_snapshot(&mut disk, &first), b"first!");
    assert_eq!(read_snapshot(&mut disk, &second), b"second");
    assert_eq!(volume.retained_records().unwrap()[0], Some(first));
    assert_eq!(volume.retained_records().unwrap()[1], Some(second));

    let generation_zero = Header7::decode(
        disk.live
            .get(&format7::header_sector(0))
            .expect("generation 0 header"),
    )
    .unwrap();
    let generation_one = Header7::decode(
        disk.live
            .get(&format7::header_sector(1))
            .expect("generation 1 header"),
    )
    .unwrap();
    assert_eq!(generation_zero.sequence, 3);
    assert_eq!(generation_zero.generation, 0);
    assert_eq!(generation_one.sequence, 2);
    assert_eq!(generation_one.generation, 1);

    let mut durable = disk.recover();
    durable.fail_at = None;
    let mut mounted = Volume7::EMPTY;
    mounted.mount_into(&mut durable).unwrap();
    assert_eq!(mounted.header().unwrap().sequence, 3);
    assert_eq!(mounted.node(5).unwrap().unwrap().version, 3);
    assert_eq!(mounted.retained_records().unwrap()[0], Some(first));
    assert_eq!(mounted.retained_records().unwrap()[1], Some(second));
    assert_eq!(read_snapshot(&mut durable, &first), b"first!");
    assert_eq!(read_snapshot(&mut durable, &second), b"second");
    let operations = durable.operations;
    assert_eq!(
        mounted.replace_tracked(&mut durable, identity(16), 1, b"first!"),
        Ok(first)
    );
    assert_eq!(durable.operations, operations);
}

#[test]
fn allocator_uses_later_contiguous_space_and_maximum_payload() {
    let mut fragmented = Sparse::default();
    let (mut nodes, records, mut map, mut header) = empty_generation(0, 1);
    header.next = 14;
    nodes[4] = named_file(5, 1, 16, b"t", b"target");
    allocate_run(&mut map, 16, 1);
    write_payload(&mut fragmented, 16, b"t");
    for index in 0..8 {
        let sector = index as u64 * 2;
        let id = index as u32 + 6;
        nodes[id as usize - 1] = named_file(id, 1, sector, b"b", &[b'b', b'0' + index as u8]);
        allocate_run(&mut map, sector, 1);
        write_payload(&mut fragmented, sector, b"b");
    }
    persist_generation(&mut fragmented, header, &nodes, &records, &map);
    fragmented.flush().unwrap();
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut fragmented).unwrap();
    let bytes = vec![0x6d; 9 * 512];
    let record = volume
        .replace_tracked(&mut fragmented, identity(18), 1, &bytes)
        .unwrap();
    assert_eq!(record.extents_used, 1);
    assert_eq!(read_snapshot(&mut fragmented, &record), bytes);

    let contiguous_bytes = vec![0x72; 9 * 512];
    let contiguous = volume
        .replace_tracked(
            &mut fragmented,
            identity(25),
            record.committed,
            &contiguous_bytes,
        )
        .unwrap();
    assert_eq!(contiguous.extents_used, 1);
    assert_eq!(
        read_snapshot(&mut fragmented, &contiguous),
        contiguous_bytes
    );
    let mut durable = fragmented.recover();
    durable.fail_at = None;
    let mut mounted = Volume7::EMPTY;
    mounted.mount_into(&mut durable).unwrap();
    assert_eq!(
        mounted.node(5).unwrap().unwrap().version,
        contiguous.committed
    );
    assert_eq!(read_snapshot(&mut durable, &record), bytes);
    assert_eq!(read_snapshot(&mut durable, &contiguous), contiguous_bytes);

    let mut maximum_disk = Sparse::default();
    seed_one_file(&mut maximum_disk, b"old");
    let mut maximum = Volume7::EMPTY;
    maximum.mount_into(&mut maximum_disk).unwrap();
    let bytes = vec![0xa5; 256 * 1024];
    let record = maximum
        .replace_tracked(&mut maximum_disk, identity(19), 1, &bytes)
        .unwrap();
    assert_eq!(record.length, 256 * 1024);
    assert_eq!(record.extents_used, 1);
    assert_eq!(read_snapshot(&mut maximum_disk, &record), bytes);
}

#[test]
fn every_payload_metadata_header_and_flush_cut_recovers_the_old_generation() {
    const PUBLICATION_STEPS: usize = 106;
    let bytes = vec![0x4d; 3 * 512];

    for fail_at in 0..PUBLICATION_STEPS {
        let mut disk = Sparse::default();
        seed_one_file(&mut disk, b"before");
        let mut volume = Volume7::EMPTY;
        volume.mount_into(&mut disk).unwrap();
        disk.operations = 0;
        disk.fail_at = Some(fail_at);
        assert_eq!(
            volume.replace_tracked(&mut disk, identity(20), 1, &bytes),
            Err(Error::Uncertain),
            "publication step {fail_at} must be uncertain"
        );
        assert_eq!(volume.header().err(), Some(Error::Uncertain));
        assert_eq!(volume.node(5).err(), Some(Error::Uncertain));

        let mut durable = disk.recover();
        durable.fail_at = None;
        let mut recovered = Volume7::EMPTY;
        recovered.mount_into(&mut durable).unwrap();
        assert_eq!(
            recovered.header().unwrap().sequence,
            1,
            "durable recovery at step {fail_at} must select the complete old head"
        );
        assert_eq!(recovered.node(5).unwrap().unwrap().version, 1);
        assert!(
            recovered
                .retained_records()
                .unwrap()
                .iter()
                .all(Option::is_none)
        );
    }
}

#[test]
fn every_reused_generation_cut_keeps_the_previous_committed_snapshot() {
    const PUBLICATION_STEPS: usize = 106;
    let first_bytes = vec![0x31; 3 * 512];
    let second_bytes = vec![0x52; 3 * 512];

    for fail_at in 0..PUBLICATION_STEPS {
        let mut disk = Sparse::default();
        seed_one_file(&mut disk, b"before");
        let mut volume = Volume7::EMPTY;
        volume.mount_into(&mut disk).unwrap();
        let first = volume
            .replace_tracked(&mut disk, identity(26), 1, &first_bytes)
            .unwrap();
        disk.operations = 0;
        disk.fail_at = Some(fail_at);

        assert_eq!(
            volume.replace_tracked(&mut disk, identity(27), first.committed, &second_bytes),
            Err(Error::Uncertain),
            "reused-generation step {fail_at} must be uncertain"
        );
        assert_eq!(volume.header().err(), Some(Error::Uncertain));

        let mut durable = disk.recover();
        durable.fail_at = None;
        let mut recovered = Volume7::EMPTY;
        recovered.mount_into(&mut durable).unwrap();
        assert_eq!(recovered.header().unwrap().sequence, first.committed);
        assert_eq!(recovered.node(5).unwrap().unwrap().version, first.committed);
        assert_eq!(recovered.retained_records().unwrap()[0], Some(first));
        assert!(recovered.retained_records().unwrap()[1].is_none());
        assert_eq!(read_snapshot(&mut durable, &first), first_bytes);
    }
}

#[test]
fn a_later_mount_flush_can_recover_the_complete_head_after_a_final_flush_error() {
    let mut disk = Sparse::default();
    seed_one_file(&mut disk, b"before");
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    disk.operations = 0;
    disk.fail_at = Some(103);

    assert_eq!(
        volume.replace_tracked(&mut disk, identity(24), 1, b"after!"),
        Err(Error::Uncertain)
    );
    assert_eq!(volume.header().err(), Some(Error::Uncertain));

    disk.fail_at = None;
    volume.mount_into(&mut disk).unwrap();
    assert_eq!(volume.header().unwrap().sequence, 2);
    let record = volume.retained_records().unwrap()[0].unwrap();
    assert_eq!(record.state, RecordState::DirectCommitted);
    assert_eq!(volume.node(5).unwrap().unwrap().version, record.committed);
    assert_eq!(read_snapshot(&mut disk, &record), b"after!");
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
fn a_final_flush_error_after_durable_completion_remounts_the_new_head() {
    let mut disk = Sparse::default();
    seed_one_file(&mut disk, b"before");
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();

    let flushes = {
        let mut failing = DurableFlushFailure {
            disk: &mut disk,
            flushes: 0,
            fail_at: 1,
        };
        assert_eq!(
            volume.replace_tracked(&mut failing, identity(28), 1, b"after!"),
            Err(Error::Uncertain)
        );
        failing.flushes
    };
    assert_eq!(flushes, 2);
    assert_eq!(volume.header().err(), Some(Error::Uncertain));

    let mut recovered = Volume7::EMPTY;
    recovered.mount_into(&mut disk).unwrap();
    assert_eq!(recovered.header().unwrap().sequence, 2);
    let record = recovered.retained_records().unwrap()[0].unwrap();
    assert_eq!(
        recovered.node(5).unwrap().unwrap().version,
        record.committed
    );
    assert_eq!(read_snapshot(&mut disk, &record), b"after!");
}

struct TornHeader<'a> {
    disk: &'a mut Sparse,
    sector: u64,
    prefix: usize,
}

impl Disk for TornHeader<'_> {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        self.disk.read(sector, bytes)
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        if sector == self.sector {
            let mut torn = self.disk.live.get(&sector).copied().unwrap_or([0; 512]);
            torn[..self.prefix].copy_from_slice(&bytes[..self.prefix]);
            self.disk.live.insert(sector, torn);
            self.disk.durable.insert(sector, torn);
            return Err(Error::Io);
        }
        self.disk.write(sector, bytes)
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.disk.flush()
    }
}

#[test]
fn torn_inactive_header_recovers_old_generation_and_reports_recovery() {
    let mut disk = Sparse::default();
    seed_one_file(&mut disk, b"before");
    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    let result = volume.replace_tracked(
        &mut TornHeader {
            disk: &mut disk,
            sector: format7::header_sector(1),
            prefix: 256,
        },
        identity(21),
        1,
        b"after!",
    );
    assert_eq!(result, Err(Error::Uncertain));
    assert_eq!(volume.header().err(), Some(Error::Uncertain));

    let mut recovered = Volume7::EMPTY;
    recovered.mount_into(&mut disk).unwrap();
    assert_eq!(recovered.header().unwrap().sequence, 1);
    assert!(recovered.recovered_from_header().unwrap());
    assert_eq!(recovered.node(5).unwrap().unwrap().version, 1);
    assert!(
        recovered
            .retained_records()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
}
