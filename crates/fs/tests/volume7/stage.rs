// SPDX-License-Identifier: Apache-2.0
//! Owner-side streamed staging: sector-at-a-time tracked replacement and
//! admission, in-memory reservations, retries, cuts and refusals.

use super::*;
use core::task::Poll;
use rustic_fs::{
    PollDisk, PollDisk7, PollPublication7, Publication7Phase, Stage7, Stage7Kind, WriteIdentity7,
};
use std::collections::HashMap;

const BEFORE: &[u8] = b"before";
/// Free holes left by the fragmented seed, largest last.
const HOLES: [(u64, u64); 3] = [(100, 2), (200, 3), (300, 4)];
const HOLE_SECTORS: u64 = 9;

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

fn pattern(seed: u8, length: usize) -> Vec<u8> {
    (0..length)
        .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn seed_one_file() -> Sparse {
    let mut disk = Sparse::default();
    let (mut nodes, records, mut map, mut header) = empty_generation(0, 1);
    header.next = 6;
    nodes[4] = file_node(5, 1, 0, BEFORE);
    allocate_run(&mut map, 0, 1);
    write_payload(&mut disk, 0, BEFORE);
    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
    disk.operations = 0;
    disk
}

/// File 5 plus `records` terminal direct records, leaving the rest free.
fn seed_with_records(records_used: usize) -> Sparse {
    let mut disk = Sparse::default();
    let (mut nodes, mut records, map, mut header) = empty_generation(0, 1 + records_used as u64);
    header.next = 6;
    nodes[4] = file_node(5, 1 + records_used as u64, 0, b"");
    nodes[4].extents_used = 0;
    nodes[4].extents = [Extent::new(0, 0); MAX_EXTENTS];
    for (index, slot) in records.iter_mut().take(records_used).enumerate() {
        let previous = index as u64 + 1;
        *slot = Some(Record7 {
            subject: 9,
            workspace: 4,
            object: 5,
            instance: 1,
            retry_epoch: 1,
            retry_key: 100 + index as u64,
            previous,
            committed: previous + 1,
            admission_number: 0,
            terminal: previous + 1,
            length: 0,
            payload_crc32: format7::aggregate(&[]),
            state: RecordState::DirectCommitted,
            prevention: None,
            extents_used: 0,
            extents: [Extent::new(0, 0); MAX_EXTENTS],
        });
    }
    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
    disk.operations = 0;
    disk
}

/// File 5 at sector 0 and zero-filled filler files owning every other payload
/// sector except [`HOLES`], so a nine-sector payload needs three runs.
fn seed_fragmented() -> Sparse {
    let mut disk = Sparse::default();
    let (mut nodes, records, mut map, mut header) = empty_generation(0, 1);
    nodes[4] = file_node(5, 1, 0, BEFORE);
    allocate_run(&mut map, 0, 1);
    write_payload(&mut disk, 0, BEFORE);

    let mut occupied = Vec::new();
    let mut cursor = 1;
    for (start, sectors) in HOLES {
        occupied.push((cursor, start));
        cursor = start + sectors;
    }
    occupied.push((cursor, DATA_SECTORS));

    let mut checksums = HashMap::new();
    let mut id = 6u32;
    for (mut start, end) in occupied {
        while start < end {
            let sectors = (end - start).min(u64::from(format7::MAX_FILE_BYTES) / 512);
            let length = sectors as usize * 512;
            let checksum = *checksums
                .entry(length)
                .or_insert_with(|| format7::aggregate(&vec![0; length]));
            let mut extents = [Extent::new(0, 0); MAX_EXTENTS];
            extents[0] = Extent::new(start, sectors);
            let name = format!("f{id}");
            nodes[id as usize - 1] = Node7 {
                id,
                parent: 2,
                version: 1,
                length: length as u32,
                kind: Kind::File,
                space: 2,
                extents_used: 1,
                extents,
                name_length: name.len() as u8,
                name: name_field(name.as_bytes()),
                payload_crc32: checksum,
            };
            allocate_run(&mut map, start, sectors);
            start += sectors;
            id += 1;
        }
    }
    header.next = id;
    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
    disk.operations = 0;
    disk
}

fn mount(disk: &mut Sparse) -> Volume7 {
    let mut volume = Volume7::EMPTY;
    volume.mount_into(disk).unwrap();
    volume
}

fn remount(disk: &Sparse) -> (Sparse, Volume7) {
    let mut durable = disk.recover();
    durable.fail_at = None;
    let volume = mount(&mut durable);
    (durable, volume)
}

/// Open a stage and supply every sector of `bytes`.
fn stream(
    volume: &mut Volume7,
    disk: &mut Sparse,
    identity: WriteIdentity7,
    expected_version: u64,
    bytes: &[u8],
    kind: Stage7Kind,
) -> Result<Stage7, Error> {
    let mut stage = volume.open_stage(identity, expected_version, bytes.len() as u32, kind)?;
    let mut remaining = bytes.len();
    for sector in bytes.chunks(512) {
        remaining -= sector.len();
        assert_eq!(
            volume.stage_write(disk, &mut stage, sector)?,
            remaining as u32
        );
    }
    Ok(stage)
}

fn read_snapshot(disk: &mut Sparse, runs: &[Extent], length: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(length as usize);
    let mut block = [0u8; 512];
    let mut remaining = length as usize;
    for run in runs {
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

fn assert_same_media(streamed: &Sparse, borrowed: &Sparse, case: &str) {
    assert_eq!(streamed.operations, borrowed.operations, "{case}: commands");
    assert!(
        streamed.live == borrowed.live,
        "{case}: live sectors differ"
    );
    assert!(
        streamed.durable == borrowed.durable,
        "{case}: durable sectors differ"
    );
}

/// Poll view of a sparse disk whose commands settle immediately.
struct Ready<'a>(&'a mut Sparse);

impl PollDisk for Ready<'_> {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        Poll::Ready(self.0.write(sector, bytes))
    }

    fn poll_flush(&mut self) -> Poll<Result<(), Error>> {
        Poll::Ready(self.0.flush())
    }
}

impl PollDisk7 for Ready<'_> {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), Error>> {
        Poll::Ready(self.0.read(sector, bytes))
    }
}

fn settle(publication: &mut PollPublication7<'_, impl PollDisk7>) -> Result<Record7, Error> {
    loop {
        match publication.poll_advance() {
            Poll::Pending => (),
            Poll::Ready(Ok(Publication7Phase::Committed)) => {
                return Ok(publication.result().unwrap());
            }
            Poll::Ready(Ok(Publication7Phase::Cancelled)) => panic!("unexpected cancellation"),
            Poll::Ready(Ok(_)) => (),
            Poll::Ready(Err(error)) => return Err(error),
        }
    }
}

struct ReadFailure<'a>(&'a mut Sparse);

impl Disk for ReadFailure<'_> {
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
fn streamed_tracked_replacement_matches_borrowed_replacement_byte_for_byte() {
    for length in [0, 1, 511, 512, 513, format7::MAX_FILE_BYTES as usize] {
        let bytes = pattern(length as u8, length);
        let case = format!("{length} bytes");

        let mut borrowed_disk = seed_one_file();
        let mut borrowed = mount(&mut borrowed_disk);
        let expected = borrowed
            .replace_tracked(&mut borrowed_disk, identity(11), 1, &bytes)
            .unwrap();

        let mut disk = seed_one_file();
        let mut volume = mount(&mut disk);
        let mut stage = stream(
            &mut volume,
            &mut disk,
            identity(11),
            1,
            &bytes,
            Stage7Kind::Tracked,
        )
        .unwrap();
        let record = volume.finish_tracked(&mut disk, &mut stage).unwrap();

        assert_eq!(record, expected, "{case}");
        assert_eq!(record.payload_crc32, format7::aggregate(&bytes), "{case}");
        assert_eq!(
            volume.header().unwrap(),
            borrowed.header().unwrap(),
            "{case}"
        );
        assert_eq!(
            volume.allocation_map().unwrap(),
            borrowed.allocation_map().unwrap(),
            "{case}"
        );
        assert_same_media(&disk, &borrowed_disk, &case);

        let (mut durable, mounted) = remount(&disk);
        assert_eq!(
            mounted.retained_records().unwrap()[0],
            Some(record),
            "{case}"
        );
        let node = *mounted.node(5).unwrap().unwrap();
        assert_eq!(node.version, record.committed, "{case}");
        assert_eq!(
            read_snapshot(&mut durable, node.runs(), node.length),
            bytes,
            "{case}"
        );
    }
}

#[test]
fn streamed_admission_matches_borrowed_admission_and_can_execute() {
    let bytes = pattern(7, 3 * 512 + 17);

    let mut borrowed_disk = seed_one_file();
    let mut borrowed = mount(&mut borrowed_disk);
    let expected = {
        let mut ready = Ready(&mut borrowed_disk);
        let mut publication = borrowed
            .prepare_admission(&mut ready, identity(13), 1, &bytes)
            .unwrap();
        settle(&mut publication).unwrap()
    };

    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);
    let mut stage = stream(
        &mut volume,
        &mut disk,
        identity(13),
        1,
        &bytes,
        Stage7Kind::Admission,
    )
    .unwrap();
    let record = {
        let mut ready = Ready(&mut disk);
        let mut publication = volume.finish_admission(&mut ready, &mut stage).unwrap();
        settle(&mut publication).unwrap()
    };

    assert_eq!(record, expected);
    assert_eq!(record.state, RecordState::Admitted);
    assert_eq!(volume.node(5).unwrap().unwrap().version, 1);
    assert_same_media(&disk, &borrowed_disk, "admission");

    let (mut durable, mut mounted) = remount(&disk);
    assert_eq!(
        read_snapshot(&mut durable, record.runs(), record.length),
        bytes
    );
    let executed = {
        let mut ready = Ready(&mut durable);
        let mut publication = mounted
            .prepare_execute(&mut ready, identity(13), 1)
            .unwrap();
        settle(&mut publication).unwrap()
    };
    assert_eq!(executed.state, RecordState::AdmittedCommitted);
    let node = *mounted.node(5).unwrap().unwrap();
    assert_eq!(read_snapshot(&mut durable, node.runs(), node.length), bytes);
}

#[test]
fn every_streamed_tracked_cut_is_uncertain_and_remounts_the_old_head() {
    // Three payload writes, then 64 node, 32 map and 4 receipt sectors, the
    // metadata flush, the header write and the final flush.
    const COMMANDS: usize = 106;
    let bytes = pattern(9, 3 * 512);

    for fail_at in 0..COMMANDS {
        let mut disk = seed_one_file();
        let mut volume = mount(&mut disk);
        let free = volume.free_sectors().unwrap();
        disk.operations = 0;
        disk.fail_at = Some(fail_at);

        let mut stage = volume
            .open_stage(identity(20), 1, bytes.len() as u32, Stage7Kind::Tracked)
            .unwrap();
        let mut failed_in_stream = false;
        for sector in bytes.chunks(512) {
            if let Err(error) = volume.stage_write(&mut disk, &mut stage, sector) {
                assert_eq!(error, Error::Uncertain, "cut {fail_at}");
                failed_in_stream = true;
                break;
            }
        }
        if failed_in_stream {
            assert!(fail_at < 3, "cut {fail_at}");
            assert_eq!(volume.header().err(), Some(Error::Uncertain));
            disk.fail_at = None;
            volume.mount_into(&mut disk.recover()).unwrap();
            // The token predates the fence and remount.
            assert_eq!(
                volume.stage_write(&mut disk, &mut stage, &bytes[..512]),
                Err(Error::Invalid),
                "cut {fail_at}"
            );
            assert_eq!(volume.abort_stage(stage), Err(Error::Invalid));
        } else {
            assert!(fail_at >= 3, "cut {fail_at}");
            assert_eq!(
                volume.finish_tracked(&mut disk, &mut stage),
                Err(Error::Uncertain),
                "cut {fail_at}"
            );
        }
        if !failed_in_stream {
            assert_eq!(volume.header().err(), Some(Error::Uncertain));
        }

        let (_, recovered) = remount(&disk);
        assert_eq!(recovered.header().unwrap().sequence, 1, "cut {fail_at}");
        assert_eq!(recovered.node(5).unwrap().unwrap().version, 1);
        assert!(
            recovered
                .retained_records()
                .unwrap()
                .iter()
                .all(Option::is_none)
        );
        assert_eq!(
            recovered.free_sectors(),
            Ok(free),
            "cut {fail_at} leaked sectors"
        );
    }
}

#[test]
fn every_streamed_admission_cut_is_uncertain_and_remounts_the_old_head() {
    // One payload write, then the same 104 publication commands as a borrowed
    // admission of one sector.
    const COMMANDS: usize = 105;
    let bytes = pattern(4, 300);

    for fail_at in 0..COMMANDS {
        let mut disk = seed_one_file();
        let mut volume = mount(&mut disk);
        let free = volume.free_sectors().unwrap();
        disk.operations = 0;
        disk.fail_at = Some(fail_at);

        let mut stage = volume
            .open_stage(identity(21), 1, bytes.len() as u32, Stage7Kind::Admission)
            .unwrap();
        match volume.stage_write(&mut disk, &mut stage, &bytes) {
            Err(error) => {
                assert_eq!((fail_at, error), (0, Error::Uncertain));
                assert_eq!(volume.header().err(), Some(Error::Uncertain));
            }
            Ok(0) => {
                let mut ready = Ready(&mut disk);
                let mut publication = volume.finish_admission(&mut ready, &mut stage).unwrap();
                assert_eq!(
                    settle(&mut publication),
                    Err(Error::Uncertain),
                    "cut {fail_at}"
                );
                drop(publication);
                assert_eq!(volume.header().err(), Some(Error::Uncertain));
            }
            Ok(remaining) => panic!("unexpected remaining {remaining}"),
        }

        let (_, recovered) = remount(&disk);
        assert_eq!(recovered.header().unwrap().sequence, 1, "cut {fail_at}");
        assert!(
            recovered
                .retained_records()
                .unwrap()
                .iter()
                .all(Option::is_none)
        );
        assert_eq!(
            recovered.free_sectors(),
            Ok(free),
            "cut {fail_at} leaked sectors"
        );
    }
}

#[test]
fn abort_mid_stream_releases_without_io_and_the_scope_can_reopen() {
    let bytes = pattern(5, 3 * 512);
    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);
    let free = volume.free_sectors().unwrap();
    let map = *volume.allocation_map().unwrap();

    let mut stage = volume
        .open_stage(identity(30), 1, bytes.len() as u32, Stage7Kind::Tracked)
        .unwrap();
    assert_eq!(
        volume.stage_write(&mut disk, &mut stage, &bytes[..512]),
        Ok(1024)
    );
    // A reservation never reaches the mounted map.
    assert_eq!(volume.free_sectors(), Ok(free));
    assert_eq!(volume.allocation_map().unwrap(), &map);
    let operations = disk.operations;
    volume.abort_stage(stage).unwrap();
    assert_eq!(disk.operations, operations);
    assert_eq!(volume.free_sectors(), Ok(free));
    assert_eq!(volume.header().unwrap().sequence, 1);

    let mut stage = stream(
        &mut volume,
        &mut disk,
        identity(30),
        1,
        &bytes,
        Stage7Kind::Tracked,
    )
    .unwrap();
    let record = volume.finish_tracked(&mut disk, &mut stage).unwrap();
    assert_eq!(
        read_snapshot(&mut disk, record.runs(), record.length),
        bytes
    );
}

#[test]
fn a_remount_makes_earlier_tokens_stale_even_when_their_slot_is_reused() {
    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);
    let mut old = volume
        .open_stage(identity(31), 1, 512, Stage7Kind::Tracked)
        .unwrap();
    volume.mount_into(&mut disk).unwrap();

    let mut new = volume
        .open_stage(identity(31), 1, 512, Stage7Kind::Tracked)
        .unwrap();
    assert_eq!(
        volume.stage_write(&mut disk, &mut old, &[1; 512]),
        Err(Error::Invalid)
    );
    assert_eq!(
        volume.finish_tracked(&mut disk, &mut old),
        Err(Error::Invalid)
    );
    assert_eq!(volume.stage_write(&mut disk, &mut new, &[2; 512]), Ok(0));
    let record = volume.finish_tracked(&mut disk, &mut new).unwrap();
    assert_eq!(read_snapshot(&mut disk, record.runs(), 512), [2; 512]);
}

#[test]
fn write_length_kind_and_completeness_are_checked() {
    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);
    let mut stage = volume
        .open_stage(identity(32), 1, 700, Stage7Kind::Tracked)
        .unwrap();
    let operations = disk.operations;
    for wrong in [0, 188, 511, 513, 700] {
        assert_eq!(
            volume.stage_write(&mut disk, &mut stage, &vec![0; wrong]),
            Err(Error::Invalid),
            "{wrong} bytes"
        );
    }
    assert_eq!(disk.operations, operations);
    // Argument refusals leave the stage usable.
    assert_eq!(
        volume.stage_write(&mut disk, &mut stage, &[3; 512]),
        Ok(188)
    );
    // An incomplete or wrong-kind finish is refused and keeps the stage open.
    assert_eq!(
        volume.finish_tracked(&mut disk, &mut stage),
        Err(Error::Invalid)
    );
    assert_eq!(volume.open_stages(), 1);
    assert_eq!(volume.stage_write(&mut disk, &mut stage, &[3; 188]), Ok(0));
    {
        let mut ready = Ready(&mut disk);
        assert_eq!(
            volume.finish_admission(&mut ready, &mut stage).err(),
            Some(Error::Invalid)
        );
    }
    assert_eq!(volume.open_stages(), 1);
    assert_eq!(disk.operations, operations + 2);
    let record = volume.finish_tracked(&mut disk, &mut stage).unwrap();
    assert_eq!(read_snapshot(&mut disk, record.runs(), 700), [3; 700]);
    assert_eq!(volume.open_stages(), 0);
    // After finishing, the token is stale.
    assert_eq!(
        volume.finish_tracked(&mut disk, &mut stage),
        Err(Error::Invalid)
    );

    let mut stage = stream(
        &mut volume,
        &mut disk,
        identity(33),
        record.committed,
        &[4; 700],
        Stage7Kind::Admission,
    )
    .unwrap();
    assert_eq!(
        volume.finish_tracked(&mut disk, &mut stage),
        Err(Error::Invalid)
    );
    assert_eq!(
        volume.stage_write(&mut disk, &mut stage, &[]),
        Err(Error::Invalid)
    );
    let mut ready = Ready(&mut disk);
    let mut publication = volume.finish_admission(&mut ready, &mut stage).unwrap();
    assert_eq!(
        settle(&mut publication).unwrap().state,
        RecordState::Admitted
    );
}

#[test]
fn a_token_from_another_owner_is_refused() {
    let mut first_disk = seed_one_file();
    let mut second_disk = seed_one_file();
    let mut first = mount(&mut first_disk);
    let mut second = mount(&mut second_disk);
    // Same slot and nonce on both owners; only the owner identity differs.
    let mut foreign = first
        .open_stage(identity(35), 1, 512, Stage7Kind::Tracked)
        .unwrap();
    let mut own = second
        .open_stage(identity(35), 1, 512, Stage7Kind::Tracked)
        .unwrap();

    assert_eq!(
        second.stage_write(&mut second_disk, &mut foreign, &[1; 512]),
        Err(Error::Invalid)
    );
    assert_eq!(
        second.finish_tracked(&mut second_disk, &mut foreign),
        Err(Error::Invalid)
    );
    assert_eq!(second.abort_stage(foreign), Err(Error::Invalid));
    assert_eq!(second.open_stages(), 1);
    assert_eq!(first.open_stages(), 1);

    assert_eq!(
        second.stage_write(&mut second_disk, &mut own, &[2; 512]),
        Ok(0)
    );
    assert!(second.finish_tracked(&mut second_disk, &mut own).is_ok());
    assert_eq!(first.header().unwrap().sequence, 1);
}

#[test]
fn release_stages_frees_reservations_without_io_and_unblocks_maintenance() {
    let mut disk = seed_with_records(RETAINED - 1);
    let mut volume = mount(&mut disk);
    let version = RETAINED as u64;
    let mut dropped = volume
        .open_stage(identity(36), version, 1024, Stage7Kind::Tracked)
        .unwrap();
    assert_eq!(
        volume.stage_write(&mut disk, &mut dropped, &[6; 512]),
        Ok(512)
    );
    let _verifying = volume
        .open_stage(identity(100), 1, 0, Stage7Kind::Tracked)
        .unwrap();
    assert_eq!(volume.open_stages(), 2);
    assert_eq!(
        volume
            .open_stage(identity(37), version, 4, Stage7Kind::Admission)
            .err(),
        Some(Error::Busy)
    );
    assert_eq!(volume.maintain_retention(&mut disk), Err(Error::Busy));

    let operations = disk.operations;
    let free = volume.free_sectors().unwrap();
    assert_eq!(volume.release_stages(), 2);
    assert_eq!(disk.operations, operations);
    assert_eq!(volume.open_stages(), 0);
    assert_eq!(volume.free_sectors(), Ok(free));
    assert!(volume.header().is_ok());
    assert_eq!(
        volume.stage_write(&mut disk, &mut dropped, &[6; 512]),
        Err(Error::Invalid)
    );

    // The receipt slot the dropped stage reserved is available again.
    let mut stage = stream(
        &mut volume,
        &mut disk,
        identity(37),
        version,
        &[7; 1024],
        Stage7Kind::Tracked,
    )
    .unwrap();
    let record = volume.finish_tracked(&mut disk, &mut stage).unwrap();
    assert_eq!(read_snapshot(&mut disk, record.runs(), 1024), [7; 1024]);
    assert_eq!(volume.release_stages(), 0);
    assert_eq!(volume.maintain_retention(&mut disk), Ok(2));
}

#[test]
fn two_fresh_stages_interleave_and_commit_to_distinct_runs_and_slots() {
    let mut disk = Sparse::default();
    let (mut nodes, records, mut map, mut header) = empty_generation(0, 1);
    header.next = 7;
    nodes[4] = file_node(5, 1, 0, BEFORE);
    nodes[5] = file_node(6, 1, 1, b"other");
    nodes[5].name = name_field(b"other");
    nodes[5].name_length = 5;
    allocate_run(&mut map, 0, 2);
    write_payload(&mut disk, 0, BEFORE);
    write_payload(&mut disk, 1, b"other");
    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
    let mut volume = mount(&mut disk);

    let first_bytes = pattern(14, 3 * 512 + 5);
    let second_bytes = pattern(15, 2 * 512 + 9);
    let second_identity = WriteIdentity7 {
        object: 6,
        ..identity(39)
    };
    let mut first = volume
        .open_stage(
            identity(38),
            1,
            first_bytes.len() as u32,
            Stage7Kind::Tracked,
        )
        .unwrap();
    let mut second = volume
        .open_stage(
            second_identity,
            1,
            second_bytes.len() as u32,
            Stage7Kind::Tracked,
        )
        .unwrap();
    let mut first_chunks = first_bytes.chunks(512);
    let mut second_chunks = second_bytes.chunks(512);
    loop {
        let a = first_chunks.next();
        let b = second_chunks.next();
        if a.is_none() && b.is_none() {
            break;
        }
        if let Some(chunk) = a {
            volume.stage_write(&mut disk, &mut first, chunk).unwrap();
        }
        if let Some(chunk) = b {
            volume.stage_write(&mut disk, &mut second, chunk).unwrap();
        }
    }
    let second_record = volume.finish_tracked(&mut disk, &mut second).unwrap();
    let first_record = volume.finish_tracked(&mut disk, &mut first).unwrap();
    assert_eq!(second_record.committed, 2);
    assert_eq!(first_record.committed, 3);
    for left in first_record.runs() {
        for right in second_record.runs() {
            assert!(left.end() <= right.start || right.end() <= left.start);
        }
    }
    // Four plus three new payload sectors are owned; both old ones were released.
    let expected_free = DATA_SECTORS - 7;
    assert_eq!(volume.free_sectors(), Ok(expected_free));

    let (mut durable, mounted) = remount(&disk);
    assert_eq!(mounted.free_sectors(), Ok(expected_free));
    let slots = mounted.retained_records().unwrap();
    assert_eq!(slots[0], Some(second_record));
    assert_eq!(slots[1], Some(first_record));
    for (id, bytes) in [(5, &first_bytes), (6, &second_bytes)] {
        let node = *mounted.node(id).unwrap().unwrap();
        assert_eq!(
            &read_snapshot(&mut durable, node.runs(), node.length),
            bytes
        );
    }
}

#[test]
fn streamed_retry_after_commit_verifies_without_writes_or_flushes() {
    let bytes = pattern(6, 2 * 512 + 3);
    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);
    let committed = volume
        .replace_tracked(&mut disk, identity(40), 1, &bytes)
        .unwrap();
    let free = volume.free_sectors().unwrap();
    let operations = disk.operations;

    let mut stage = stream(
        &mut volume,
        &mut disk,
        identity(40),
        1,
        &bytes,
        Stage7Kind::Tracked,
    )
    .unwrap();
    assert_eq!(volume.finish_tracked(&mut disk, &mut stage), Ok(committed));
    assert_eq!(disk.operations, operations);
    assert_eq!(volume.free_sectors(), Ok(free));
    assert_eq!(volume.header().unwrap().sequence, 2);

    let mut altered = bytes.clone();
    altered[600] ^= 1;
    let mut stage = stream(
        &mut volume,
        &mut disk,
        identity(40),
        1,
        &altered,
        Stage7Kind::Tracked,
    )
    .unwrap();
    assert_eq!(
        volume.finish_tracked(&mut disk, &mut stage),
        Err(Error::IdempotencyConflict)
    );
    assert_eq!(disk.operations, operations);
    assert!(volume.header().is_ok());

    // Metadata conflicts refuse at open, before any read.
    assert_eq!(
        volume
            .open_stage(identity(40), 1, bytes.len() as u32 + 1, Stage7Kind::Tracked)
            .err(),
        Some(Error::IdempotencyConflict)
    );
}

#[test]
fn streamed_retry_reports_corruption_before_a_byte_difference() {
    let bytes = pattern(8, 2 * 512);
    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);
    let committed = volume
        .replace_tracked(&mut disk, identity(41), 1, &bytes)
        .unwrap();
    disk.corrupt(
        format7::PAYLOAD_SECTOR + committed.runs()[0].start + 1,
        700 - 512,
    );
    let operations = disk.operations;

    let mut altered = bytes.clone();
    altered[3] ^= 1;
    let mut stage = stream(
        &mut volume,
        &mut disk,
        identity(41),
        1,
        &altered,
        Stage7Kind::Tracked,
    )
    .unwrap();
    assert_eq!(
        volume.finish_tracked(&mut disk, &mut stage),
        Err(Error::Corrupt)
    );
    assert_eq!(disk.operations, operations);
    assert_eq!(volume.header().unwrap().sequence, 2);
}

#[test]
fn streamed_retry_read_failure_releases_the_stage_without_fencing() {
    let bytes = pattern(10, 512 + 1);
    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);
    volume
        .replace_tracked(&mut disk, identity(42), 1, &bytes)
        .unwrap();
    let operations = disk.operations;

    let mut stage = volume
        .open_stage(identity(42), 1, bytes.len() as u32, Stage7Kind::Tracked)
        .unwrap();
    assert_eq!(
        volume.stage_write(&mut ReadFailure(&mut disk), &mut stage, &bytes[..512]),
        Err(Error::Io)
    );
    assert_eq!(disk.operations, operations);
    assert_eq!(volume.header().unwrap().sequence, 2);
    assert_eq!(
        volume.stage_write(&mut disk, &mut stage, &bytes[..512]),
        Err(Error::Invalid)
    );
    assert_eq!(volume.maintain_retention(&mut disk).map(|_| ()), Ok(()));
}

#[test]
fn streamed_admission_retry_replays_the_current_record_without_io() {
    let bytes = pattern(11, 900);
    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);
    let admitted = {
        let mut stage = stream(
            &mut volume,
            &mut disk,
            identity(43),
            1,
            &bytes,
            Stage7Kind::Admission,
        )
        .unwrap();
        let mut ready = Ready(&mut disk);
        let mut publication = volume.finish_admission(&mut ready, &mut stage).unwrap();
        settle(&mut publication).unwrap()
    };

    let mut stage = stream(
        &mut volume,
        &mut disk,
        identity(43),
        1,
        &bytes,
        Stage7Kind::Admission,
    )
    .unwrap();
    // Execution completes while the verifying stage is open.
    let executed = {
        let mut ready = Ready(&mut disk);
        let mut publication = volume.prepare_execute(&mut ready, identity(43), 1).unwrap();
        settle(&mut publication).unwrap()
    };
    assert_eq!(executed.extents, admitted.extents);
    let operations = disk.operations;
    let mut ready = Ready(&mut disk);
    let mut publication = volume.finish_admission(&mut ready, &mut stage).unwrap();
    assert_eq!(publication.phase(), Publication7Phase::Committed);
    assert_eq!(settle(&mut publication), Ok(executed));
    drop(publication);
    assert_eq!(disk.operations, operations);

    let mut altered = bytes.clone();
    altered[899] ^= 1;
    let mut stage = stream(
        &mut volume,
        &mut disk,
        identity(43),
        1,
        &altered,
        Stage7Kind::Admission,
    )
    .unwrap();
    let mut ready = Ready(&mut disk);
    assert_eq!(
        volume.finish_admission(&mut ready, &mut stage).err(),
        Some(Error::IdempotencyConflict)
    );
    assert!(volume.header().is_ok());
}

#[test]
fn fragmented_reservations_exclude_other_planners_and_three_runs_remount() {
    let mut disk = seed_fragmented();
    let mut volume = mount(&mut disk);
    assert_eq!(volume.free_sectors(), Ok(HOLE_SECTORS));
    let whole = HOLE_SECTORS as u32 * 512;
    assert_eq!(
        volume
            .open_stage(identity(50), 1, whole + 1, Stage7Kind::Tracked)
            .err(),
        Some(Error::Full)
    );

    // One stage reserving every free sector leaves none for any other planner,
    // although the mounted map still reports them free.
    let first = volume
        .open_stage(identity(50), 1, whole, Stage7Kind::Tracked)
        .unwrap();
    assert_eq!(volume.free_sectors(), Ok(HOLE_SECTORS));
    assert_eq!(
        volume
            .open_stage(identity(51), 1, 1, Stage7Kind::Admission)
            .err(),
        Some(Error::Full)
    );
    assert_eq!(
        volume
            .replace_tracked(&mut disk, identity(52), 1, b"x")
            .err(),
        Some(Error::Full)
    );
    {
        let mut ready = Ready(&mut disk);
        assert_eq!(
            volume
                .prepare_admission(&mut ready, identity(53), 1, b"x")
                .err(),
            Some(Error::Full)
        );
    }
    volume.abort_stage(first).unwrap();

    // After abort the whole hole set streams as three runs with a partial tail.
    let bytes = pattern(3, whole as usize - 100);
    let mut stage = stream(
        &mut volume,
        &mut disk,
        identity(12),
        1,
        &bytes,
        Stage7Kind::Tracked,
    )
    .unwrap();
    let record = volume.finish_tracked(&mut disk, &mut stage).unwrap();
    assert_eq!(record.extents_used, 3);
    let mut runs: Vec<_> = record
        .runs()
        .iter()
        .map(|run| (run.start, run.sectors))
        .collect();
    runs.sort_unstable();
    assert_eq!(runs, HOLES);
    // Only the released previous sector is free again.
    assert_eq!(volume.free_sectors(), Ok(1));

    let (mut durable, mounted) = remount(&disk);
    assert_eq!(mounted.retained_records().unwrap()[0], Some(record));
    let node = *mounted.node(5).unwrap().unwrap();
    assert_eq!(node.version, record.committed);
    assert_eq!(read_snapshot(&mut durable, node.runs(), node.length), bytes);
}

#[test]
fn a_remount_releases_reservations_held_by_a_forgotten_token() {
    let mut disk = seed_with_records(RETAINED - 1);
    let mut volume = mount(&mut disk);
    let version = RETAINED as u64;
    let _forgotten = volume
        .open_stage(identity(55), version, 4, Stage7Kind::Tracked)
        .unwrap();
    assert_eq!(
        volume
            .open_stage(identity(56), version, 4, Stage7Kind::Tracked)
            .err(),
        Some(Error::Full)
    );
    volume.mount_into(&mut disk).unwrap();
    let mut stage = stream(
        &mut volume,
        &mut disk,
        identity(56),
        version,
        b"four",
        Stage7Kind::Tracked,
    )
    .unwrap();
    assert!(volume.finish_tracked(&mut disk, &mut stage).is_ok());
}

#[test]
fn a_direct_replacement_during_a_stage_plans_around_its_reservation() {
    let staged = pattern(12, 3 * 512);
    let direct = pattern(13, 3 * 512);
    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);

    let mut stage = volume
        .open_stage(identity(60), 1, staged.len() as u32, Stage7Kind::Tracked)
        .unwrap();
    let committed = volume
        .replace_tracked(&mut disk, identity(61), 1, &direct)
        .unwrap();
    for sector in staged.chunks(512) {
        volume.stage_write(&mut disk, &mut stage, sector).unwrap();
    }
    // Staged writes did not land in the committed snapshot.
    assert_eq!(
        read_snapshot(&mut disk, committed.runs(), committed.length),
        direct
    );

    let free = volume.free_sectors().unwrap();
    let operations = disk.operations;
    assert_eq!(
        volume.finish_tracked(&mut disk, &mut stage),
        Err(Error::Version)
    );
    assert_eq!(disk.operations, operations);
    assert_eq!(volume.free_sectors(), Ok(free));
    assert_eq!(volume.header().unwrap().sequence, committed.committed);

    let (mut durable, mounted) = remount(&disk);
    assert_eq!(mounted.free_sectors(), Ok(free));
    let node = *mounted.node(5).unwrap().unwrap();
    assert_eq!(
        read_snapshot(&mut durable, node.runs(), node.length),
        direct
    );
}

#[test]
fn removal_between_open_and_finish_refuses_without_fencing() {
    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);
    let created = volume.create(&mut disk, 2, b"doc", Kind::File).unwrap();
    let target = WriteIdentity7 {
        object: created.id,
        ..identity(62)
    };
    let mut stage = volume
        .open_stage(target, created.version, 10, Stage7Kind::Tracked)
        .unwrap();
    volume.stage_write(&mut disk, &mut stage, &[7; 10]).unwrap();
    volume.remove(&mut disk, created.id).unwrap();
    let sequence = volume.header().unwrap().sequence;

    assert_eq!(
        volume.finish_tracked(&mut disk, &mut stage),
        Err(Error::NotFound)
    );
    assert_eq!(volume.header().unwrap().sequence, sequence);
    assert!(
        volume
            .retained_records()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn stage_capacity_scope_and_retention_maintenance_are_busy() {
    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);
    let first = volume
        .open_stage(identity(70), 1, 1, Stage7Kind::Tracked)
        .unwrap();
    for kind in [Stage7Kind::Tracked, Stage7Kind::Admission] {
        assert_eq!(
            volume.open_stage(identity(70), 1, 1, kind).err(),
            Some(Error::Busy)
        );
    }
    assert_eq!(
        volume
            .replace_tracked(&mut disk, identity(70), 1, b"x")
            .err(),
        Some(Error::Busy)
    );
    {
        let mut ready = Ready(&mut disk);
        assert_eq!(
            volume
                .prepare_admission(&mut ready, identity(70), 1, b"x")
                .err(),
            Some(Error::Busy)
        );
    }
    let second = volume
        .open_stage(identity(71), 1, 1, Stage7Kind::Admission)
        .unwrap();
    assert_eq!(
        volume
            .open_stage(identity(72), 1, 1, Stage7Kind::Tracked)
            .err(),
        Some(Error::Busy)
    );
    let operations = disk.operations;
    assert_eq!(volume.maintain_retention(&mut disk), Err(Error::Busy));
    assert_eq!(disk.operations, operations);

    volume.abort_stage(first).unwrap();
    assert_eq!(volume.maintain_retention(&mut disk), Err(Error::Busy));
    volume.abort_stage(second).unwrap();
    assert_eq!(volume.maintain_retention(&mut disk), Ok(2));
}

#[test]
fn a_fresh_stage_reserves_a_receipt_slot() {
    let mut disk = seed_with_records(RETAINED - 1);
    let mut volume = mount(&mut disk);
    let version = RETAINED as u64;
    let stage = volume
        .open_stage(identity(80), version, 4, Stage7Kind::Admission)
        .unwrap();
    assert_eq!(
        volume
            .open_stage(identity(81), version, 4, Stage7Kind::Tracked)
            .err(),
        Some(Error::Full)
    );
    assert_eq!(
        volume
            .replace_tracked(&mut disk, identity(82), version, b"late")
            .err(),
        Some(Error::Full)
    );
    // A verifying retry needs no slot.
    let mut retry = volume
        .open_stage(identity(100), 1, 0, Stage7Kind::Tracked)
        .unwrap();
    assert_eq!(
        volume
            .finish_tracked(&mut disk, &mut retry)
            .map(|record| record.retry_key),
        Ok(100)
    );
    volume.abort_stage(stage).unwrap();
    assert!(
        volume
            .replace_tracked(&mut disk, identity(82), version, b"late")
            .is_ok()
    );
}

#[test]
fn open_stage_applies_the_replacement_preflight_without_io() {
    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);
    disk.operations = 0;
    for (identity, version, length, expected) in [
        (identity(90), 2, 1, Error::Version),
        (
            WriteIdentity7 {
                retry_epoch: 2,
                ..identity(91)
            },
            1,
            1,
            Error::ExpiredEpoch,
        ),
        (
            WriteIdentity7 {
                subject: 0,
                ..identity(92)
            },
            1,
            1,
            Error::Invalid,
        ),
        (identity(93), 1, format7::MAX_FILE_BYTES + 1, Error::Size),
        (
            WriteIdentity7 {
                object: 1,
                ..identity(94)
            },
            1,
            1,
            Error::Invalid,
        ),
    ] {
        for kind in [Stage7Kind::Tracked, Stage7Kind::Admission] {
            assert_eq!(
                volume.open_stage(identity, version, length, kind).err(),
                Some(expected),
                "{expected:?} {kind:?}"
            );
        }
    }
    assert_eq!(disk.operations, 0);
    // Refusals leave no stage behind.
    assert_eq!(volume.maintain_retention(&mut disk), Ok(2));
}
