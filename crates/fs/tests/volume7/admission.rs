// SPDX-License-Identifier: Apache-2.0
//! Durable v7 admission, explicit cancellation, execution, and poll settlement.

use super::*;
use core::task::Poll;
use rustic_fs::{
    PollDisk, PollDisk7, PollPublication7, PreventionReason, Publication7Cancel, Publication7Phase,
    WriteIdentity7,
};
use std::{cell::Cell, rc::Rc};

const OLD: &[u8] = b"old-live";
const CANDIDATE: &[u8] = b"new-snapshot";

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

fn seed_file() -> Sparse {
    let mut disk = Sparse::default();
    let (mut nodes, records, mut map, mut header) = empty_generation(0, 1);
    header.next = 6;
    nodes[4] = file_node(5, 1, 0, OLD);
    allocate_run(&mut map, 0, 1);
    write_payload(&mut disk, 0, OLD);
    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
    disk.operations = 0;
    disk
}

fn seed_file_with_retained_live_snapshot() -> Sparse {
    let mut disk = Sparse::default();
    let (mut nodes, mut records, mut map, mut header) = empty_generation(0, 2);
    header.next = 6;
    nodes[4] = file_node(5, 2, 0, OLD);
    records[0] = Some(committed_snapshot(0, OLD));
    allocate_run(&mut map, 0, 1);
    write_payload(&mut disk, 0, OLD);
    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
    disk.operations = 0;
    disk
}

fn seed_exhausted_sequence() -> Sparse {
    let mut disk = Sparse::default();
    let (mut nodes, records, mut map, mut header) = empty_generation(0, u64::MAX);
    header.next = 6;
    nodes[4] = file_node(5, 1, 0, OLD);
    allocate_run(&mut map, 0, 1);
    write_payload(&mut disk, 0, OLD);
    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
    disk.operations = 0;
    disk
}

fn seed_full_receipts() -> Sparse {
    let mut disk = Sparse::default();
    let (mut nodes, mut records, map, mut header) = empty_generation(0, 9);
    header.next = 6;
    nodes[4] = file_node(5, 9, 0, b"");
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
    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
    disk.operations = 0;
    disk
}

fn seed_full_payload() -> Sparse {
    let mut disk = Sparse::default();
    let (mut nodes, mut records, mut map, mut header) = empty_generation(0, 5);
    header.next = NODES as u32 + 1;
    let zeros = vec![0; format7::MAX_FILE_BYTES as usize];
    let checksum = format7::aggregate(&zeros);
    let mut cursor = 0;

    for index in 0..NODES - 4 {
        let id = index as u32 + 5;
        let run = Extent::new(cursor, format7::MAX_FILE_BYTES as u64 / 512);
        nodes[index + 4] = full_file_node(id, &[run], checksum);
        allocate_run(&mut map, run.start, run.sectors);
        cursor += run.sectors;
    }
    for (index, record) in records.iter_mut().take(4).enumerate() {
        let run = Extent::new(cursor, format7::MAX_FILE_BYTES as u64 / 512);
        *record = Some(admitted_snapshot(index, run, checksum));
        allocate_run(&mut map, run.start, run.sectors);
        cursor += run.sectors;
    }
    assert_eq!(cursor, DATA_SECTORS);

    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
    disk.operations = 0;
    disk
}

fn seed_fragmented_payload() -> Sparse {
    let mut disk = Sparse::default();
    let (mut nodes, mut records, mut map, mut header) = empty_generation(0, 4);
    header.next = NODES as u32 + 1;
    let zeros = vec![0; format7::MAX_FILE_BYTES as usize];
    let checksum = format7::aggregate(&zeros);
    let mut runs = Vec::with_capacity(NODES - 1);
    let mut tail = 519;

    for separator in 0..7 {
        let marker = Extent::new(separator * 65 + 64, 1);
        let rest = Extent::new(tail, format7::MAX_FILE_BYTES as u64 / 512 - 1);
        runs.push([marker, rest]);
        tail += rest.sectors;
    }
    for _ in 7..NODES - 1 {
        let rest = Extent::new(tail, format7::MAX_FILE_BYTES as u64 / 512);
        runs.push([rest, Extent::new(0, 0)]);
        tail += rest.sectors;
    }
    assert_eq!(tail, DATA_SECTORS);

    for index in 0..NODES - 4 {
        let id = index as u32 + 5;
        let used = if runs[index][1].sectors == 0 { 1 } else { 2 };
        nodes[index + 4] = full_file_node(id, &runs[index][..used], checksum);
        for run in &runs[index][..used] {
            allocate_run(&mut map, run.start, run.sectors);
        }
    }
    for index in 0..3 {
        let run = runs[NODES - 4 + index][0];
        records[index] = Some(admitted_snapshot(index, run, checksum));
        allocate_run(&mut map, run.start, run.sectors);
    }

    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
    disk.operations = 0;
    disk
}

fn full_file_node(id: u32, runs: &[Extent], checksum: u32) -> Node7 {
    let mut extents = [Extent::new(0, 0); MAX_EXTENTS];
    extents[..runs.len()].copy_from_slice(runs);
    let name = format!("f{id}");
    Node7 {
        id,
        parent: 1,
        version: 1,
        length: format7::MAX_FILE_BYTES,
        kind: Kind::File,
        space: 1,
        extents_used: runs.len() as u8,
        extents,
        name_length: name.len() as u8,
        name: name_field(name.as_bytes()),
        payload_crc32: checksum,
    }
}

fn admitted_snapshot(index: usize, run: Extent, checksum: u32) -> Record7 {
    let mut extents = [Extent::new(0, 0); MAX_EXTENTS];
    extents[0] = run;
    Record7 {
        subject: 9,
        workspace: 4,
        object: 5,
        instance: 1,
        retry_epoch: 1,
        retry_key: 100 + index as u64,
        previous: 1,
        committed: 0,
        admission_number: 2 + index as u64,
        terminal: 0,
        length: format7::MAX_FILE_BYTES,
        payload_crc32: checksum,
        state: RecordState::Admitted,
        prevention: None,
        extents_used: 1,
        extents,
    }
}

fn live_bytes(disk: &mut Sparse, node: &Node7) -> Vec<u8> {
    let mut output = Vec::with_capacity(node.length as usize);
    let mut remaining = node.length as usize;
    let mut block = [0; 512];
    for run in node.runs() {
        for offset in 0..run.sectors {
            if remaining == 0 {
                return output;
            }
            disk.read(format7::PAYLOAD_SECTOR + run.start + offset, &mut block)
                .unwrap();
            let count = remaining.min(block.len());
            output.extend_from_slice(&block[..count]);
            remaining -= count;
        }
    }
    assert_eq!(remaining, 0);
    output
}

#[derive(Default)]
struct ReadyPoll {
    disk: Sparse,
    commands: usize,
    trace: Vec<PollCommand>,
    reads: usize,
    writes: usize,
    flushes: usize,
    read_error_at: Option<usize>,
    durable_flush_error: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PollCommand {
    Read(u64),
    Write(u64),
    Flush,
}

impl PollDisk for ReadyPoll {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        self.commands += 1;
        self.writes += 1;
        self.trace.push(PollCommand::Write(sector));
        Poll::Ready(self.disk.write(sector, bytes))
    }

    fn poll_flush(&mut self) -> Poll<Result<(), Error>> {
        self.commands += 1;
        self.trace.push(PollCommand::Flush);
        let index = self.flushes;
        self.flushes += 1;
        let result = self.disk.flush();
        if result.is_ok() && self.durable_flush_error == Some(index) {
            Poll::Ready(Err(Error::Io))
        } else {
            Poll::Ready(result)
        }
    }
}

impl PollDisk7 for ReadyPoll {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), Error>> {
        self.commands += 1;
        self.trace.push(PollCommand::Read(sector));
        let index = self.reads;
        self.reads += 1;
        if self.read_error_at == Some(index) {
            Poll::Ready(Err(Error::Io))
        } else {
            Poll::Ready(self.disk.read(sector, bytes))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HeldCommand {
    Read { sector: u64, result: [u8; 512] },
    Write { sector: u64, bytes: [u8; 512] },
    Flush,
}

struct GatePoll {
    disk: Sparse,
    commands: usize,
    pending: Option<HeldCommand>,
    release: Rc<Cell<bool>>,
    hold_sector: Option<u64>,
    hold_read_sector: Option<u64>,
    hold_flush_index: Option<usize>,
    flush_count: usize,
    held_once: bool,
    panic_after_submit: bool,
}

impl GatePoll {
    fn first_command(disk: Sparse) -> (Self, Rc<Cell<bool>>) {
        let release = Rc::new(Cell::new(false));
        (
            Self {
                disk,
                commands: 0,
                pending: None,
                release: release.clone(),
                hold_sector: None,
                hold_read_sector: None,
                hold_flush_index: None,
                flush_count: 0,
                held_once: false,
                panic_after_submit: false,
            },
            release,
        )
    }

    fn header(disk: Sparse, sector: u64) -> (Self, Rc<Cell<bool>>) {
        let release = Rc::new(Cell::new(false));
        (
            Self {
                disk,
                commands: 0,
                pending: None,
                release: release.clone(),
                hold_sector: Some(sector),
                hold_read_sector: None,
                hold_flush_index: None,
                flush_count: 0,
                held_once: false,
                panic_after_submit: false,
            },
            release,
        )
    }

    fn read(disk: Sparse, sector: u64) -> (Self, Rc<Cell<bool>>) {
        let release = Rc::new(Cell::new(false));
        (
            Self {
                disk,
                commands: 0,
                pending: None,
                release: release.clone(),
                hold_sector: None,
                hold_read_sector: Some(sector),
                hold_flush_index: None,
                flush_count: 0,
                held_once: false,
                panic_after_submit: false,
            },
            release,
        )
    }

    fn final_flush(disk: Sparse) -> (Self, Rc<Cell<bool>>) {
        let release = Rc::new(Cell::new(false));
        (
            Self {
                disk,
                commands: 0,
                pending: None,
                release: release.clone(),
                hold_sector: None,
                hold_read_sector: None,
                hold_flush_index: Some(2),
                flush_count: 0,
                held_once: false,
                panic_after_submit: false,
            },
            release,
        )
    }

    fn is_held(&self, command: &HeldCommand) -> bool {
        if self.held_once {
            return false;
        }
        if self.hold_flush_index.is_some() {
            return matches!(command, HeldCommand::Flush)
                && self.hold_flush_index == Some(self.flush_count);
        }
        match (self.hold_sector, self.hold_read_sector, command) {
            (None, None, _) => true,
            (Some(expected), _, HeldCommand::Write { sector, .. }) => expected == *sector,
            (_, Some(expected), HeldCommand::Read { sector, .. }) => expected == *sector,
            _ => false,
        }
    }

    fn resolve_pending(&mut self, command: &HeldCommand) -> bool {
        assert_eq!(
            self.pending.as_ref(),
            Some(command),
            "pending command changed"
        );
        if !self.release.get() {
            return false;
        }
        self.pending = None;
        true
    }
}

impl PollDisk for GatePoll {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        self.commands += 1;
        let command = HeldCommand::Write {
            sector,
            bytes: *bytes,
        };
        if self.pending.is_some() {
            return if self.resolve_pending(&command) {
                Poll::Ready(Ok(()))
            } else {
                Poll::Pending
            };
        }
        if self.is_held(&command) {
            if let Err(error) = self.disk.write(sector, bytes) {
                return Poll::Ready(Err(error));
            }
            self.held_once = true;
            self.pending = Some(command);
            assert!(!self.panic_after_submit, "adapter unwind after submission");
            Poll::Pending
        } else {
            Poll::Ready(self.disk.write(sector, bytes))
        }
    }

    fn poll_flush(&mut self) -> Poll<Result<(), Error>> {
        self.commands += 1;
        if self.pending.is_some() {
            return if self.resolve_pending(&HeldCommand::Flush) {
                Poll::Ready(Ok(()))
            } else {
                Poll::Pending
            };
        }
        let command = HeldCommand::Flush;
        let should_hold = self.is_held(&command);
        self.flush_count += 1;
        if should_hold {
            if let Err(error) = self.disk.flush() {
                return Poll::Ready(Err(error));
            }
            self.held_once = true;
            self.pending = Some(command);
            assert!(!self.panic_after_submit, "adapter unwind after submission");
            Poll::Pending
        } else {
            Poll::Ready(self.disk.flush())
        }
    }
}

impl PollDisk7 for GatePoll {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), Error>> {
        self.commands += 1;
        if let Some(HeldCommand::Read {
            sector: expected,
            result,
        }) = self.pending.as_ref()
        {
            assert_eq!(*expected, sector);
            let result = *result;
            if !self.release.get() {
                return Poll::Pending;
            }
            self.pending = None;
            *bytes = result;
            return Poll::Ready(Ok(()));
        }
        assert!(self.pending.is_none(), "pending command changed kind");
        let mut result = [0; 512];
        if let Err(error) = self.disk.read(sector, &mut result) {
            return Poll::Ready(Err(error));
        }
        let command = HeldCommand::Read { sector, result };
        if self.is_held(&command) {
            self.held_once = true;
            self.pending = Some(command);
            assert!(!self.panic_after_submit, "adapter unwind after submission");
            Poll::Pending
        } else {
            let HeldCommand::Read { result, .. } = command else {
                unreachable!()
            };
            *bytes = result;
            Poll::Ready(Ok(()))
        }
    }
}

fn mount(disk: &mut Sparse) -> Volume7 {
    let mut volume = Volume7::EMPTY;
    volume.mount_into(disk).unwrap();
    volume
}

fn settle(publication: &mut PollPublication7<'_, impl PollDisk7>) -> Record7 {
    loop {
        match publication.poll_advance() {
            Poll::Pending => (),
            Poll::Ready(Ok(Publication7Phase::Committed)) => return publication.result().unwrap(),
            Poll::Ready(Ok(
                Publication7Phase::Retrying
                | Publication7Phase::Preparing
                | Publication7Phase::ReadyToPublish
                | Publication7Phase::Settling,
            )) => (),
            Poll::Ready(Ok(phase)) => panic!("unexpected settled phase {phase:?}"),
            Poll::Ready(Err(error)) => panic!("publication failed: {error:?}"),
        }
    }
}

fn admit(volume: &mut Volume7, disk: &mut ReadyPoll, key: u64) -> Record7 {
    let mut operation = volume
        .prepare_admission(disk, identity(key), 1, CANDIDATE)
        .unwrap();
    settle(&mut operation)
}

fn durable_admission(key: u64) -> (Sparse, Record7) {
    let mut sparse = seed_file();
    let mut volume = mount(&mut sparse);
    let mut disk = ReadyPoll {
        disk: sparse,
        ..ReadyPoll::default()
    };
    let record = admit(&mut volume, &mut disk, key);
    (disk.disk.recover(), record)
}

#[test]
fn admission_flushes_candidate_before_publishing_without_changing_live_file() {
    let mut disk = seed_file();
    let mut volume = mount(&mut disk);
    disk.operations = 0;
    let old = *volume.node(5).unwrap().unwrap();
    let mut poll = ReadyPoll {
        disk,
        ..ReadyPoll::default()
    };
    let mut operation = volume
        .prepare_admission(&mut poll, identity(11), 1, CANDIDATE)
        .unwrap();
    assert_eq!(operation.result(), None);
    let record = settle(&mut operation);
    drop(operation);

    assert_eq!(record.state, RecordState::Admitted);
    assert_eq!(record.admission_number, 2);
    assert_eq!(record.committed, 0);
    assert_eq!(*volume.node(5).unwrap().unwrap(), old);
    assert_eq!(volume.header().unwrap().sequence, 2);
    assert_eq!(poll.flushes, 3, "payload, metadata, then header barriers");
    let mut expected_trace = vec![
        PollCommand::Write(format7::PAYLOAD_SECTOR + record.runs()[0].start),
        PollCommand::Flush,
    ];
    expected_trace.extend(
        (0..format7::NODES_SECTORS)
            .map(|offset| PollCommand::Write(format7::nodes_sector(1) + offset)),
    );
    expected_trace.extend(
        (0..format7::MAP_SECTORS).map(|offset| PollCommand::Write(format7::map_sector(1) + offset)),
    );
    expected_trace.extend(
        (0..format7::RECEIPTS_SECTORS)
            .map(|offset| PollCommand::Write(format7::receipts_sector(1) + offset)),
    );
    expected_trace.extend([
        PollCommand::Flush,
        PollCommand::Write(format7::header_sector(1)),
        PollCommand::Flush,
    ]);
    assert_eq!(poll.trace, expected_trace);
    assert_eq!(
        live_bytes(&mut poll.disk, volume.node(5).unwrap().unwrap()),
        OLD
    );
    assert_eq!(live_bytes_for_record(&mut poll.disk, &record), CANDIDATE);

    let mut durable = poll.disk.recover();
    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut durable).unwrap();
    assert_eq!(remounted.header().unwrap().sequence, 2);
    assert_eq!(*remounted.node(5).unwrap().unwrap(), old);
    assert_eq!(remounted.retained_records().unwrap()[0], Some(record));
    assert_eq!(
        remounted.retained_records().unwrap()[0].unwrap().state,
        RecordState::Admitted
    );
    assert_eq!(
        live_bytes(&mut durable, remounted.node(5).unwrap().unwrap()),
        OLD
    );
}

#[test]
fn empty_admission_keeps_a_canonical_zero_extent_snapshot() {
    let mut disk = seed_file();
    let mut volume = mount(&mut disk);
    let old = *volume.node(5).unwrap().unwrap();
    let mut poll = ReadyPoll {
        disk,
        ..ReadyPoll::default()
    };
    let mut operation = volume
        .prepare_admission(&mut poll, identity(55), 1, b"")
        .unwrap();
    let record = settle(&mut operation);
    drop(operation);

    assert_eq!(record.state, RecordState::Admitted);
    assert_eq!(record.length, 0);
    assert_eq!(record.extents_used, 0);
    assert_eq!(record.runs(), &[]);
    assert_eq!(*volume.node(5).unwrap().unwrap(), old);
    assert_eq!(
        poll.writes,
        format7::NODES_SECTORS as usize
            + format7::MAP_SECTORS as usize
            + format7::RECEIPTS_SECTORS as usize
            + 1
    );

    let mut durable = poll.disk.recover();
    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut durable).unwrap();
    assert_eq!(remounted.retained_records().unwrap()[0], Some(record));
    assert_eq!(*remounted.node(5).unwrap().unwrap(), old);
}

#[test]
fn fragmented_multisector_admission_and_retry_cover_partial_final_sector() {
    let bytes = vec![0xa5; format7::MAX_FILE_BYTES as usize - 17];
    let mut sparse = seed_fragmented_payload();
    let mut volume = mount(&mut sparse);
    assert_eq!(volume.free_sectors(), Ok(512));
    let old = *volume.node(5).unwrap().unwrap();
    let mut poll = ReadyPoll {
        disk: sparse,
        ..ReadyPoll::default()
    };

    let mut admission = volume
        .prepare_admission(&mut poll, identity(56), 1, &bytes)
        .unwrap();
    let record = settle(&mut admission);
    drop(admission);
    assert_eq!(record.length as usize, bytes.len());
    assert_eq!(record.runs().len(), MAX_EXTENTS);
    assert!(record.runs().iter().all(|run| run.sectors == 64));
    assert_eq!(
        record.runs().iter().map(|run| run.sectors).sum::<u64>(),
        512
    );
    assert_eq!(*volume.node(5).unwrap().unwrap(), old);

    let writes = poll.writes;
    let flushes = poll.flushes;
    let commands = poll.commands;
    let mut retry = volume
        .prepare_admission(&mut poll, identity(56), 1, &bytes)
        .unwrap();
    assert_eq!(retry.phase(), Publication7Phase::Retrying);
    assert_eq!(retry.result(), None);
    for index in 0..512 {
        let result = retry.poll_advance();
        if index < 511 {
            assert_eq!(result, Poll::Ready(Ok(Publication7Phase::Retrying)));
            assert_eq!(retry.result(), None);
        } else {
            assert_eq!(result, Poll::Ready(Ok(Publication7Phase::Committed)));
            assert_eq!(retry.result(), Some(record));
        }
    }
    drop(retry);
    assert_eq!(poll.commands - commands, 512);
    assert_eq!(poll.writes, writes);
    assert_eq!(poll.flushes, flushes);
}

#[test]
fn later_retry_corruption_wins_over_an_earlier_byte_mismatch() {
    let bytes = vec![0x4b; 1025];
    let mut disk = seed_file();
    let mut volume = mount(&mut disk);
    let mut poll = ReadyPoll {
        disk,
        ..ReadyPoll::default()
    };
    let mut admission = volume
        .prepare_admission(&mut poll, identity(57), 1, &bytes)
        .unwrap();
    let record = settle(&mut admission);
    drop(admission);
    let damaged_sector = format7::PAYLOAD_SECTOR + record.runs()[0].start + 2;
    poll.disk.corrupt(damaged_sector, 0);
    let mut changed = bytes;
    changed[0] ^= 1;

    let mut retry = volume
        .prepare_admission(&mut poll, identity(57), 1, &changed)
        .unwrap();
    assert_eq!(retry.result(), None);
    assert_eq!(
        retry.poll_advance(),
        Poll::Ready(Ok(Publication7Phase::Retrying))
    );
    assert_eq!(retry.result(), None);
    assert_eq!(
        retry.poll_advance(),
        Poll::Ready(Ok(Publication7Phase::Retrying))
    );
    assert_eq!(retry.result(), None);
    assert_eq!(retry.poll_advance(), Poll::Ready(Err(Error::Corrupt)));
    assert_eq!(retry.result(), None);
    drop(retry);
    assert_eq!(volume.header().err(), Some(Error::Uncertain));
}

fn live_bytes_for_record(disk: &mut Sparse, record: &Record7) -> Vec<u8> {
    let mut output = Vec::with_capacity(record.length as usize);
    let mut remaining = record.length as usize;
    let mut block = [0; 512];
    for run in record.runs() {
        for offset in 0..run.sectors {
            if remaining == 0 {
                return output;
            }
            disk.read(format7::PAYLOAD_SECTOR + run.start + offset, &mut block)
                .unwrap();
            let count = remaining.min(block.len());
            output.extend_from_slice(&block[..count]);
            remaining -= count;
        }
    }
    assert_eq!(remaining, 0);
    output
}

#[test]
fn exact_retry_after_remount_verifies_one_sector_per_poll_and_conflicts_on_bytes_or_identity() {
    let mut disk = seed_file();
    let mut volume = mount(&mut disk);
    let mut poll = ReadyPoll {
        disk,
        ..ReadyPoll::default()
    };
    let expected = admit(&mut volume, &mut poll, 12);
    let mut durable = poll.disk.recover();
    let mut remounted = mount(&mut durable);
    let mut retry_disk = ReadyPoll {
        disk: durable,
        ..ReadyPoll::default()
    };
    let before = retry_disk.commands;
    let mut retry = remounted
        .prepare_admission(&mut retry_disk, identity(12), 1, CANDIDATE)
        .unwrap();
    assert_eq!(retry.phase(), Publication7Phase::Retrying);
    assert_eq!(retry.result(), None);
    assert_eq!(
        retry.poll_advance(),
        Poll::Ready(Ok(Publication7Phase::Committed))
    );
    assert_eq!(retry.result(), Some(expected));
    drop(retry);
    assert_eq!(retry_disk.commands - before, 1);
    let writes = retry_disk.writes;
    let flushes = retry_disk.flushes;
    assert_eq!(retry_disk.writes, writes);
    assert_eq!(retry_disk.flushes, flushes);

    let operations = retry_disk.commands;
    let mut changed_bytes = remounted
        .prepare_admission(&mut retry_disk, identity(12), 1, b"different!!!")
        .unwrap();
    assert_eq!(changed_bytes.result(), None);
    assert_eq!(
        changed_bytes.poll_advance(),
        Poll::Ready(Err(Error::IdempotencyConflict))
    );
    assert_eq!(changed_bytes.result(), None);
    drop(changed_bytes);
    assert_eq!(retry_disk.writes, writes);
    assert_eq!(retry_disk.flushes, flushes);
    assert_eq!(retry_disk.commands, operations + 1);

    assert_eq!(
        remounted
            .prepare_admission(
                &mut retry_disk,
                WriteIdentity7 {
                    instance: 2,
                    ..identity(12)
                },
                1,
                CANDIDATE,
            )
            .err(),
        Some(Error::IdempotencyConflict)
    );
}

#[test]
fn pending_retry_read_drains_after_cancel_using_adapter_owned_result() {
    let (durable, record) = durable_admission(58);
    let sector = format7::PAYLOAD_SECTOR + record.runs()[0].start;
    let (mut disk, release) = GatePoll::read(durable, sector);
    let mut volume = mount(&mut disk.disk);
    let mut retry = volume
        .prepare_admission(&mut disk, identity(58), 1, CANDIDATE)
        .unwrap();

    assert_eq!(retry.poll_advance(), Poll::Pending);
    assert!(retry.pending());
    let mut retry = retry;
    assert_eq!(
        retry.abort_before_header(),
        Ok(Publication7Cancel::Draining)
    );
    assert_eq!(retry.poll_advance(), Poll::Pending);
    release.set(true);
    assert_eq!(
        retry.poll_advance(),
        Poll::Ready(Ok(Publication7Phase::Cancelled))
    );
    assert_eq!(retry.result(), None);
    drop(retry);
    assert_eq!(volume.header().unwrap().sequence, 2);
    assert_eq!(volume.retained_records().unwrap()[0], Some(record));
}

#[test]
fn dropping_a_pending_retry_read_fences_until_remount_recovers_the_record() {
    let (durable, record) = durable_admission(59);
    let sector = format7::PAYLOAD_SECTOR + record.runs()[0].start;
    let (mut disk, _release) = GatePoll::read(durable, sector);
    let mut volume = mount(&mut disk.disk);
    let mut retry = volume
        .prepare_admission(&mut disk, identity(59), 1, CANDIDATE)
        .unwrap();

    assert_eq!(retry.poll_advance(), Poll::Pending);
    drop(retry);
    assert_eq!(volume.header().err(), Some(Error::Uncertain));

    let mut recovered = disk.disk.recover();
    volume.mount_into(&mut recovered).unwrap();
    assert_eq!(volume.header().unwrap().sequence, 2);
    assert_eq!(volume.retained_records().unwrap()[0], Some(record));
}

#[test]
fn retry_read_error_fences_until_mount_reestablishes_the_durable_admission() {
    let mut sparse = seed_file();
    let mut volume = mount(&mut sparse);
    let mut disk = ReadyPoll {
        disk: sparse,
        ..ReadyPoll::default()
    };
    let record = admit(&mut volume, &mut disk, 16);
    disk.read_error_at = Some(0);
    let writes = disk.writes;
    let flushes = disk.flushes;
    let mut retry = volume
        .prepare_admission(&mut disk, identity(16), 1, CANDIDATE)
        .unwrap();
    assert_eq!(retry.result(), None);
    assert_eq!(retry.poll_advance(), Poll::Ready(Err(Error::Uncertain)));
    drop(retry);
    assert_eq!(volume.header().err(), Some(Error::Uncertain));
    assert_eq!(disk.writes, writes);
    assert_eq!(disk.flushes, flushes);

    disk.disk.fail_at = None;
    let mut durable = disk.disk.recover();
    volume.mount_into(&mut durable).unwrap();
    assert_eq!(volume.header().unwrap().sequence, 2);
    assert_eq!(volume.retained_records().unwrap()[0], Some(record));
}

#[test]
fn explicit_execute_commits_candidate_releases_only_unowned_old_runs_and_remounts() {
    let mut disk = seed_file();
    let mut volume = mount(&mut disk);
    let old = *volume.node(5).unwrap().unwrap();
    let mut poll = ReadyPoll {
        disk,
        ..ReadyPoll::default()
    };
    let admitted = admit(&mut volume, &mut poll, 13);
    let mut execute = volume.prepare_execute(&mut poll, identity(13), 1).unwrap();
    assert_eq!(execute.result(), None);
    let committed = settle(&mut execute);
    drop(execute);

    assert_eq!(committed.state, RecordState::AdmittedCommitted);
    assert_eq!(committed.admission_number, admitted.admission_number);
    assert_eq!(committed.committed, 3);
    assert_eq!(committed.terminal, 3);
    assert_eq!(volume.node(5).unwrap().unwrap().version, 3);
    assert_eq!(volume.node(5).unwrap().unwrap().runs(), committed.runs());
    assert_eq!(volume.allocation_map().unwrap()[0] & 1, 0);
    assert!(volume.allocation_map().unwrap()[0] & (1 << committed.runs()[0].start) != 0);
    assert_eq!(
        live_bytes(&mut poll.disk, volume.node(5).unwrap().unwrap()),
        CANDIDATE
    );
    assert_eq!(live_bytes_for_record(&mut poll.disk, &committed), CANDIDATE);

    let mut durable = poll.disk.recover();
    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut durable).unwrap();
    assert_eq!(remounted.header().unwrap().sequence, 3);
    assert_eq!(remounted.node(5).unwrap().unwrap().version, 3);
    assert_eq!(remounted.retained_records().unwrap()[0], Some(committed));
    assert_eq!(
        live_bytes(&mut durable, remounted.node(5).unwrap().unwrap()),
        CANDIDATE
    );
    assert_eq!(live_bytes_for_record(&mut durable, &committed), CANDIDATE);
    assert_eq!(old.version, 1);
}

#[test]
fn explicit_execute_keeps_old_extents_owned_by_another_retained_record() {
    let mut disk = seed_file_with_retained_live_snapshot();
    let mut volume = mount(&mut disk);
    let mut poll = ReadyPoll {
        disk,
        ..ReadyPoll::default()
    };
    assert_eq!(volume.node(5).unwrap().unwrap().version, 2);
    assert_eq!(
        volume.retained_records().unwrap()[0].unwrap().state,
        RecordState::DirectCommitted
    );

    let mut admission = volume
        .prepare_admission(&mut poll, identity(42), 2, CANDIDATE)
        .unwrap();
    let admitted = settle(&mut admission);
    drop(admission);
    let mut execution = volume.prepare_execute(&mut poll, identity(42), 2).unwrap();
    let committed = settle(&mut execution);
    drop(execution);

    assert_eq!(committed.state, RecordState::AdmittedCommitted);
    assert_ne!(committed.runs()[0].start, 0);
    assert_eq!(volume.node(5).unwrap().unwrap().version, 4);
    assert_ne!(
        volume.allocation_map().unwrap()[0] & 1,
        0,
        "the earlier committed snapshot still owns the old run"
    );
    assert_eq!(
        volume.retained_records().unwrap()[0],
        Some(committed_snapshot(0, OLD))
    );
    assert_eq!(volume.retained_records().unwrap()[1], Some(committed));
    assert_eq!(admitted.state, RecordState::Admitted);

    let mut durable = poll.disk.recover();
    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut durable).unwrap();
    assert_eq!(remounted.node(5).unwrap().unwrap().version, 4);
    assert!(remounted.allocation_map().unwrap()[0] & 1 != 0);
    assert_eq!(
        live_bytes(&mut durable, remounted.node(5).unwrap().unwrap()),
        CANDIDATE
    );
    assert_eq!(
        live_bytes_for_record(&mut durable, &committed_snapshot(0, OLD)),
        OLD
    );
}

#[test]
fn explicit_cancellation_retains_cause_and_candidate_without_live_effect_on_remount() {
    let mut disk = seed_file();
    let mut volume = mount(&mut disk);
    let old = *volume.node(5).unwrap().unwrap();
    let mut poll = ReadyPoll {
        disk,
        ..ReadyPoll::default()
    };
    let admitted = admit(&mut volume, &mut poll, 14);
    let mut cancellation = volume
        .prepare_cancellation(&mut poll, identity(14), 1, PreventionReason::AuthorityLost)
        .unwrap();
    assert_eq!(cancellation.result(), None);
    let cancelled = settle(&mut cancellation);
    drop(cancellation);

    assert_eq!(cancelled.state, RecordState::Cancelled);
    assert_eq!(cancelled.prevention, Some(PreventionReason::AuthorityLost));
    assert_eq!(cancelled.admission_number, admitted.admission_number);
    assert_eq!(cancelled.terminal, 3);
    assert_eq!(*volume.node(5).unwrap().unwrap(), old);
    assert_eq!(
        live_bytes(&mut poll.disk, volume.node(5).unwrap().unwrap()),
        OLD
    );
    assert_eq!(live_bytes_for_record(&mut poll.disk, &cancelled), CANDIDATE);

    let mut durable = poll.disk.recover();
    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut durable).unwrap();
    assert_eq!(remounted.node(5).unwrap().unwrap().version, 1);
    assert_eq!(remounted.retained_records().unwrap()[0], Some(cancelled));
    assert_eq!(
        live_bytes(&mut durable, remounted.node(5).unwrap().unwrap()),
        OLD
    );
    assert_eq!(live_bytes_for_record(&mut durable, &cancelled), CANDIDATE);

    let before = poll.commands;
    assert_eq!(
        remounted
            .prepare_cancellation(&mut poll, identity(14), 1, PreventionReason::AuthorityLost,)
            .unwrap()
            .result(),
        Some(cancelled)
    );
    assert_eq!(poll.commands, before);
    assert_eq!(
        remounted
            .prepare_cancellation(&mut poll, identity(14), 1, PreventionReason::Requested,)
            .err(),
        Some(Error::IdempotencyConflict)
    );
}

#[test]
fn execute_rechecks_the_live_version_and_version_conflict_cancellation_is_durable() {
    let mut disk = seed_file();
    let mut volume = mount(&mut disk);
    let mut poll = ReadyPoll {
        disk,
        ..ReadyPoll::default()
    };
    let admitted = admit(&mut volume, &mut poll, 43);
    assert_eq!(admitted.state, RecordState::Admitted);

    let competing = volume
        .replace_tracked(&mut poll.disk, identity(44), 1, b"competitor")
        .unwrap();
    assert_eq!(competing.state, RecordState::DirectCommitted);
    let command_count = poll.commands;
    assert_eq!(
        volume.prepare_execute(&mut poll, identity(43), 1).err(),
        Some(Error::Version)
    );
    assert_eq!(poll.commands, command_count);
    assert_eq!(
        volume.node(5).unwrap().unwrap().version,
        competing.committed
    );

    let mut cancellation = volume
        .prepare_cancellation(
            &mut poll,
            identity(43),
            1,
            PreventionReason::VersionConflict,
        )
        .unwrap();
    let cancelled = settle(&mut cancellation);
    drop(cancellation);
    assert_eq!(cancelled.state, RecordState::Cancelled);
    assert_eq!(
        cancelled.prevention,
        Some(PreventionReason::VersionConflict)
    );
    assert_eq!(
        volume.node(5).unwrap().unwrap().version,
        competing.committed
    );

    let mut durable = poll.disk.recover();
    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut durable).unwrap();
    assert_eq!(remounted.retained_records().unwrap()[0], Some(cancelled));
    assert_eq!(remounted.retained_records().unwrap()[1], Some(competing));
    assert_eq!(
        remounted.node(5).unwrap().unwrap().version,
        competing.committed
    );
    assert_eq!(
        live_bytes(&mut durable, remounted.node(5).unwrap().unwrap()),
        b"competitor"
    );
}

#[test]
fn admitted_and_cancelled_snapshots_retain_ownership_and_block_maintenance_until_terminal() {
    let mut disk = seed_file();
    let mut volume = mount(&mut disk);
    let mut poll = ReadyPoll {
        disk,
        ..ReadyPoll::default()
    };
    let admitted = admit(&mut volume, &mut poll, 15);
    let map = *volume.allocation_map().unwrap();
    let operations = poll.disk.operations;
    assert_eq!(volume.maintain_retention(&mut poll.disk), Err(Error::Busy));
    assert_eq!(poll.disk.operations, operations);
    assert_eq!(*volume.allocation_map().unwrap(), map);
    assert_eq!(volume.retained_records().unwrap()[0], Some(admitted));

    let mut cancellation = volume
        .prepare_cancellation(&mut poll, identity(15), 1, PreventionReason::Requested)
        .unwrap();
    let cancelled = settle(&mut cancellation);
    drop(cancellation);
    assert_eq!(cancelled.state, RecordState::Cancelled);
    let snapshot_sector = cancelled.runs()[0].start;
    assert!(
        volume.allocation_map().unwrap()[snapshot_sector as usize / 64]
            & (1 << (snapshot_sector % 64))
            != 0
    );
}

#[test]
fn full_and_identity_version_epoch_size_refusals_do_no_poll_io() {
    let mut disk = seed_file();
    let mut volume = mount(&mut disk);
    disk.operations = 0;
    let mut poll = ReadyPoll {
        disk,
        ..ReadyPoll::default()
    };
    assert_eq!(
        volume
            .prepare_admission(&mut poll, identity(21), 2, CANDIDATE)
            .err(),
        Some(Error::Version)
    );
    assert_eq!(
        volume
            .prepare_admission(
                &mut poll,
                WriteIdentity7 {
                    subject: 0,
                    ..identity(22)
                },
                1,
                CANDIDATE,
            )
            .err(),
        Some(Error::Invalid)
    );
    assert_eq!(
        volume
            .prepare_admission(
                &mut poll,
                WriteIdentity7 {
                    retry_epoch: 2,
                    ..identity(23)
                },
                1,
                CANDIDATE,
            )
            .err(),
        Some(Error::ExpiredEpoch)
    );
    let oversized = vec![0; format7::MAX_FILE_BYTES as usize + 1];
    assert_eq!(
        volume
            .prepare_admission(&mut poll, identity(24), 1, &oversized)
            .err(),
        Some(Error::Size)
    );
    assert_eq!(poll.commands, 0);
    assert_eq!(poll.disk.operations, 0);

    assert_eq!(
        volume
            .prepare_admission(
                &mut poll,
                WriteIdentity7 {
                    instance: u64::MAX,
                    ..identity(27)
                },
                1,
                CANDIDATE,
            )
            .err(),
        Some(Error::Invalid)
    );
    assert_eq!(poll.commands, 0);
    assert_eq!(poll.disk.operations, 0);

    drop(poll);
    let mut full_disk = seed_full_receipts();
    let mut full = mount(&mut full_disk);
    full_disk.operations = 0;
    let mut poll = ReadyPoll {
        disk: full_disk,
        ..ReadyPoll::default()
    };
    assert_eq!(
        full.prepare_admission(&mut poll, identity(25), 9, b"never")
            .err(),
        Some(Error::Full)
    );
    assert_eq!(poll.commands, 0);
    assert_eq!(poll.disk.operations, 0);

    drop(poll);
    let mut exhausted_disk = seed_full_payload();
    let mut exhausted = mount(&mut exhausted_disk);
    assert_eq!(exhausted.free_sectors(), Ok(0));
    exhausted_disk.operations = 0;
    let mut poll = ReadyPoll {
        disk: exhausted_disk,
        ..ReadyPoll::default()
    };
    assert_eq!(
        exhausted
            .prepare_admission(&mut poll, identity(26), 1, b"payload space is full")
            .err(),
        Some(Error::Full)
    );
    assert_eq!(poll.commands, 0);
    assert_eq!(poll.disk.operations, 0);
}

#[test]
fn sequence_overflow_refuses_before_poll_io() {
    let mut disk = seed_exhausted_sequence();
    let mut volume = mount(&mut disk);
    disk.operations = 0;
    let mut poll = ReadyPoll {
        disk,
        ..ReadyPoll::default()
    };
    assert_eq!(
        volume
            .prepare_admission(&mut poll, identity(28), 1, CANDIDATE)
            .err(),
        Some(Error::Exhausted)
    );
    assert_eq!(poll.commands, 0);
    assert_eq!(poll.disk.operations, 0);
}

#[test]
fn preheader_cancel_drains_one_pending_command_before_confirming_no_effect() {
    let mut sparse = seed_file();
    let mut volume = mount(&mut sparse);
    let old = *volume.node(5).unwrap().unwrap();
    let (mut disk, release) = GatePoll::first_command(sparse);
    let mut operation = volume
        .prepare_admission(&mut disk, identity(31), 1, CANDIDATE)
        .unwrap();
    assert_eq!(operation.poll_advance(), Poll::Pending);
    assert!(operation.pending());
    assert_eq!(
        operation.abort_before_header(),
        Ok(Publication7Cancel::Draining)
    );
    assert_eq!(operation.result(), None);
    assert_eq!(operation.poll_advance(), Poll::Pending);
    release.set(true);
    assert_eq!(
        operation.poll_advance(),
        Poll::Ready(Ok(Publication7Phase::Cancelled))
    );
    assert!(!operation.pending());
    drop(operation);
    assert_eq!(*volume.node(5).unwrap().unwrap(), old);
    assert_eq!(volume.header().unwrap().sequence, 1);

    let mut recovered = disk.disk.recover();
    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut recovered).unwrap();
    assert_eq!(remounted.header().unwrap().sequence, 1);
    assert!(
        remounted
            .retained_records()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
    assert_eq!(
        live_bytes(&mut recovered, remounted.node(5).unwrap().unwrap()),
        OLD
    );
}

#[test]
fn header_submission_is_too_late_to_cancel_and_settles_new_admission() {
    let mut sparse = seed_file();
    let mut volume = mount(&mut sparse);
    let (mut disk, release) = GatePoll::header(sparse, format7::header_sector(1));
    let mut operation = volume
        .prepare_admission(&mut disk, identity(32), 1, CANDIDATE)
        .unwrap();
    while operation.phase() != Publication7Phase::ReadyToPublish {
        assert!(matches!(operation.poll_advance(), Poll::Ready(Ok(_))));
    }
    assert_eq!(operation.poll_advance(), Poll::Pending);
    assert_eq!(operation.phase(), Publication7Phase::Settling);
    assert_eq!(
        operation.abort_before_header(),
        Ok(Publication7Cancel::TooLate)
    );
    release.set(true);
    assert_eq!(
        operation.poll_advance(),
        Poll::Ready(Ok(Publication7Phase::Settling))
    );
    assert_eq!(
        operation.poll_advance(),
        Poll::Ready(Ok(Publication7Phase::Committed))
    );
    let record = operation.result().unwrap();
    drop(operation);
    assert_eq!(record.state, RecordState::Admitted);
    assert_eq!(volume.header().unwrap().sequence, 2);
}

#[test]
fn pending_final_flush_is_too_late_to_cancel_and_drop_requires_remount() {
    let mut sparse = seed_file();
    let mut volume = mount(&mut sparse);
    let (mut disk, _release) = GatePoll::final_flush(sparse);
    let mut operation = volume
        .prepare_admission(&mut disk, identity(35), 1, CANDIDATE)
        .unwrap();

    loop {
        match operation.poll_advance() {
            Poll::Pending => break,
            Poll::Ready(Ok(_)) => (),
            Poll::Ready(Err(error)) => panic!("unexpected poll failure: {error:?}"),
        }
    }
    assert_eq!(operation.phase(), Publication7Phase::Settling);
    assert_eq!(operation.result(), None);
    assert_eq!(
        operation.abort_before_header(),
        Ok(Publication7Cancel::TooLate)
    );
    assert_eq!(operation.result(), None);
    drop(operation);
    assert_eq!(disk.pending, Some(HeldCommand::Flush));
    assert_eq!(volume.header().err(), Some(Error::Uncertain));

    let mut durable = disk.disk.recover();
    volume.mount_into(&mut durable).unwrap();
    assert_eq!(volume.header().unwrap().sequence, 2);
    assert_eq!(
        volume.retained_records().unwrap()[0].unwrap().state,
        RecordState::Admitted
    );
    assert_eq!(volume.node(5).unwrap().unwrap().version, 1);
}

#[test]
fn dropping_or_unwinding_with_pending_io_fences_until_successful_remount() {
    for unwind in [false, true] {
        let mut sparse = seed_file();
        let mut volume = mount(&mut sparse);
        let (mut disk, _) = GatePoll::first_command(sparse);
        disk.panic_after_submit = unwind;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut operation = volume
                .prepare_admission(
                    &mut disk,
                    identity(if unwind { 34 } else { 33 }),
                    1,
                    CANDIDATE,
                )
                .unwrap();
            let result = operation.poll_advance();
            if unwind {
                assert!(matches!(result, Poll::Pending));
            } else {
                assert_eq!(result, Poll::Pending);
            }
        }));
        assert_eq!(result.is_err(), unwind);
        assert_eq!(volume.header().err(), Some(Error::Uncertain));

        let mut recovered = disk.disk.recover();
        volume.mount_into(&mut recovered).unwrap();
        assert_eq!(volume.header().unwrap().sequence, 1);
        assert_eq!(
            live_bytes(&mut recovered, volume.node(5).unwrap().unwrap()),
            OLD
        );
    }
}

#[test]
fn admission_write_and_flush_cut_sweep_remounts_the_old_head() {
    const COMMANDS: usize = 105;
    for fail_at in 0..COMMANDS {
        let mut sparse = seed_file();
        let mut volume = mount(&mut sparse);
        sparse.operations = 0;
        sparse.fail_at = Some(fail_at);
        let mut disk = ReadyPoll {
            disk: sparse,
            ..ReadyPoll::default()
        };
        let mut operation = volume
            .prepare_admission(&mut disk, identity(40), 1, CANDIDATE)
            .unwrap();
        loop {
            match operation.poll_advance() {
                Poll::Pending | Poll::Ready(Ok(_)) => (),
                Poll::Ready(Err(error)) => {
                    assert_eq!(error, Error::Uncertain, "cut {fail_at}");
                    break;
                }
            }
        }
        drop(operation);
        assert_eq!(
            volume.header().err(),
            Some(Error::Uncertain),
            "cut {fail_at}"
        );

        disk.disk.fail_at = None;
        let mut durable = disk.disk.recover();
        volume.mount_into(&mut durable).unwrap();
        assert_eq!(volume.header().unwrap().sequence, 1, "cut {fail_at}");
        assert!(
            volume
                .retained_records()
                .unwrap()
                .iter()
                .all(Option::is_none)
        );
        assert_eq!(
            live_bytes(&mut durable, volume.node(5).unwrap().unwrap()),
            OLD
        );
    }
}

#[test]
fn execute_and_cancellation_write_cut_sweeps_keep_the_admitted_head() {
    const COMMANDS: usize = 103;
    for cancel in [false, true] {
        for fail_at in 0..COMMANDS {
            let mut sparse = seed_file();
            let mut volume = mount(&mut sparse);
            let mut setup = ReadyPoll {
                disk: sparse,
                ..ReadyPoll::default()
            };
            admit(&mut volume, &mut setup, 41);
            let mut sparse = setup.disk;
            sparse.operations = 0;
            sparse.fail_at = Some(fail_at);
            let mut disk = ReadyPoll {
                disk: sparse,
                ..ReadyPoll::default()
            };
            let mut operation = if cancel {
                volume
                    .prepare_cancellation(&mut disk, identity(41), 1, PreventionReason::Requested)
                    .unwrap()
            } else {
                volume.prepare_execute(&mut disk, identity(41), 1).unwrap()
            };
            loop {
                match operation.poll_advance() {
                    Poll::Pending | Poll::Ready(Ok(_)) => (),
                    Poll::Ready(Err(error)) => {
                        assert_eq!(error, Error::Uncertain, "cancel={cancel} cut={fail_at}");
                        break;
                    }
                }
            }
            drop(operation);
            assert_eq!(volume.header().err(), Some(Error::Uncertain));

            disk.disk.fail_at = None;
            let mut durable = disk.disk.recover();
            volume.mount_into(&mut durable).unwrap();
            assert_eq!(volume.header().unwrap().sequence, 2, "cut={fail_at}");
            assert_eq!(
                volume.retained_records().unwrap()[0].unwrap().state,
                RecordState::Admitted,
                "cancel={cancel} cut={fail_at}"
            );
            assert_eq!(volume.node(5).unwrap().unwrap().version, 1);
            assert_eq!(
                live_bytes(&mut durable, volume.node(5).unwrap().unwrap()),
                OLD
            );
        }
    }
}

#[test]
fn final_flush_error_after_durable_admission_recovers_the_new_head() {
    let mut sparse = seed_file();
    let mut volume = mount(&mut sparse);
    let mut disk = ReadyPoll {
        disk: sparse,
        durable_flush_error: Some(2),
        ..ReadyPoll::default()
    };
    let mut operation = volume
        .prepare_admission(&mut disk, identity(50), 1, CANDIDATE)
        .unwrap();
    loop {
        match operation.poll_advance() {
            Poll::Pending | Poll::Ready(Ok(_)) => (),
            Poll::Ready(Err(error)) => {
                assert_eq!(error, Error::Uncertain);
                break;
            }
        }
    }
    drop(operation);
    assert_eq!(volume.header().err(), Some(Error::Uncertain));
    let mut durable = disk.disk.recover();
    volume.mount_into(&mut durable).unwrap();
    assert_eq!(volume.header().unwrap().sequence, 2);
    let record = volume.retained_records().unwrap()[0].unwrap();
    assert_eq!(record.state, RecordState::Admitted);
    assert_eq!(volume.node(5).unwrap().unwrap().version, 1);
    assert_eq!(live_bytes_for_record(&mut durable, &record), CANDIDATE);
}

#[test]
fn final_flush_error_after_execute_or_cancel_recovers_the_terminal_head() {
    for cancel in [false, true] {
        let key = if cancel { 52 } else { 51 };
        let mut sparse = seed_file();
        let mut volume = mount(&mut sparse);
        let old_node = *volume.node(5).unwrap().unwrap();
        let mut disk = ReadyPoll {
            disk: sparse,
            ..ReadyPoll::default()
        };
        let admitted = admit(&mut volume, &mut disk, key);
        let admitted_map = *volume.allocation_map().unwrap();
        disk.durable_flush_error = Some(disk.flushes + 1);

        let mut operation = if cancel {
            volume
                .prepare_cancellation(&mut disk, identity(key), 1, PreventionReason::Requested)
                .unwrap()
        } else {
            volume.prepare_execute(&mut disk, identity(key), 1).unwrap()
        };
        loop {
            match operation.poll_advance() {
                Poll::Pending | Poll::Ready(Ok(_)) => (),
                Poll::Ready(Err(error)) => {
                    assert_eq!(error, Error::Uncertain, "cancel={cancel}");
                    break;
                }
            }
        }
        assert_eq!(operation.result(), None);
        drop(operation);
        assert_eq!(volume.header().err(), Some(Error::Uncertain));

        let mut durable = disk.disk.recover();
        volume.mount_into(&mut durable).unwrap();
        assert_eq!(volume.header().unwrap().sequence, 3);
        let terminal = volume.retained_records().unwrap()[0].unwrap();
        assert_eq!(terminal.admission_number, admitted.admission_number);
        assert_eq!(terminal.terminal, 3);
        assert_eq!(
            terminal.state,
            if cancel {
                RecordState::Cancelled
            } else {
                RecordState::AdmittedCommitted
            }
        );
        assert_eq!(
            terminal.committed,
            if cancel { 0 } else { 3 },
            "cancel={cancel}"
        );
        assert_eq!(
            terminal.prevention,
            if cancel {
                Some(PreventionReason::Requested)
            } else {
                None
            }
        );

        let node = volume.node(5).unwrap().unwrap();
        assert_eq!(node.version, if cancel { old_node.version } else { 3 });
        assert_eq!(
            live_bytes(&mut durable, node),
            if cancel { OLD } else { CANDIDATE }
        );
        if cancel {
            assert_eq!(*volume.allocation_map().unwrap(), admitted_map);
        } else {
            let mut expected_map = admitted_map;
            for run in old_node.runs() {
                for sector in run.start..run.end() {
                    expected_map[sector as usize / 64] &= !(1u64 << (sector % 64));
                }
            }
            assert_eq!(*volume.allocation_map().unwrap(), expected_map);
        }
        for run in terminal.runs() {
            for sector in run.start..run.end() {
                assert_ne!(
                    volume.allocation_map().unwrap()[sector as usize / 64]
                        & (1u64 << (sector % 64)),
                    0,
                    "cancel={cancel} sector={sector}"
                );
            }
        }
    }
}
