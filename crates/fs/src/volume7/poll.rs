// SPDX-License-Identifier: Apache-2.0
//! Bounded v7 pollable reads and dual-generation publication.

use crate::checksum::crc_update;
use crate::extent::Extent;
use crate::format7::{
    Header7, MAP_SECTORS, MAX_EXTENTS, NODE_BYTES, NODES_SECTORS, RECEIPTS_SECTORS, RECORD_BYTES,
    RETAINED, Record7, header_sector, map_sector, nodes_sector, receipts_sector,
    validate_generation,
};
use crate::{Error, PollDisk, format7::Node7};
use core::task::Poll;

use super::Volume7;
use super::payload::PayloadPlan;

/// A v7 poll adapter adds one-sector reads for exact retry verification.
///
/// The destination is borrowed only for the duration of this call. On `Pending`,
/// the adapter must retain the command and sector-sized result in adapter-owned
/// storage; it must not retain the destination pointer. When the command settles,
/// it copies the result into the destination supplied to that poll call. This is
/// required because the publication guard and its scratch sector may move or be
/// dropped while I/O remains in flight. Each call to
/// [`PollPublication7::poll_advance`] issues at most one read, write, or flush
/// command.
pub trait PollDisk7: PollDisk {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), Error>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Publication7Phase {
    Retrying,
    Preparing,
    ReadyToPublish,
    /// The header may have been submitted; the final flush has not settled.
    Settling,
    Committed,
    Cancelled,
    Failed,
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Publication7Cancel {
    Cancelled,
    TooLate,
    Draining,
}

#[derive(Clone, Copy)]
pub(super) struct Candidate7 {
    pub(super) record_slot: usize,
    pub(super) record: Record7,
    pub(super) node_update: Option<(usize, Node7)>,
    pub(super) added: PayloadPlan,
    pub(super) released: [Extent; MAX_EXTENTS],
    pub(super) released_used: usize,
}

impl Candidate7 {
    pub(super) fn empty(record_slot: usize, record: Record7) -> Self {
        Self {
            record_slot,
            record,
            node_update: None,
            added: PayloadPlan::empty(),
            released: [Extent::new(0, 0); MAX_EXTENTS],
            released_used: 0,
        }
    }

    fn release_runs(&self) -> &[Extent] {
        &self.released[..self.released_used]
    }
}

#[derive(Clone, Copy)]
enum Step {
    RetryRead(u64),
    PayloadWrite(u64),
    PayloadFlush,
    NodesWrite(u64),
    MapWrite(u64),
    ReceiptsWrite(u64),
    MetadataFlush,
    HeaderWrite,
    HeaderFlush,
    Done,
}

/// A single v7 retry check or storage publication. It borrows the owner, disk,
/// and (for admission or retry checking) caller bytes until the transition has
/// settled. Candidate structures are small deltas; no file-sized owner copy is
/// made, and only one sector of command scratch is retained.
#[must_use = "poll to settlement or abort before the header is submitted"]
pub struct PollPublication7<'a, D> {
    volume: &'a mut Volume7,
    disk: &'a mut D,
    candidate: Option<Candidate7>,
    bytes: Option<&'a [u8]>,
    retry_record: Option<Record7>,
    retry_matches: bool,
    retry_checksum: u32,
    payload_offset: usize,
    step: Step,
    phase: Publication7Phase,
    result: Option<Record7>,
    error: Option<Error>,
    pending: bool,
    stopping: bool,
    io_started: bool,
    block_prepared: bool,
    block: [u8; 512],
    nodes_checksum: u32,
    map_checksum: u32,
    receipts_checksum: u32,
    published_header: Option<Header7>,
}

impl<'a, D> PollPublication7<'a, D> {
    pub(super) fn publish(
        volume: &'a mut Volume7,
        disk: &'a mut D,
        candidate: Candidate7,
        bytes: Option<&'a [u8]>,
    ) -> Self {
        volume.fenced = true;
        let step = if bytes.is_some() && candidate.added_sectors() != 0 {
            Step::PayloadWrite(0)
        } else if bytes.is_some() {
            Step::PayloadFlush
        } else {
            Step::NodesWrite(0)
        };
        Self {
            volume,
            disk,
            candidate: Some(candidate),
            bytes,
            retry_record: None,
            retry_matches: true,
            retry_checksum: !0u32,
            payload_offset: 0,
            step,
            phase: Publication7Phase::Preparing,
            result: None,
            error: None,
            pending: false,
            stopping: false,
            io_started: false,
            block_prepared: false,
            block: [0; 512],
            nodes_checksum: !0u32,
            map_checksum: !0u32,
            receipts_checksum: !0u32,
            published_header: None,
        }
    }

    /// Publish a candidate whose payload an owner-side stage already wrote.
    /// The first command is the payload flush, so the barrier sequence and
    /// cut points after the payload writes match [`Self::publish`].
    pub(super) fn staged(volume: &'a mut Volume7, disk: &'a mut D, candidate: Candidate7) -> Self {
        let mut publication = Self::publish(volume, disk, candidate, None);
        publication.step = Step::PayloadFlush;
        publication
    }

    pub(super) fn retry(
        volume: &'a mut Volume7,
        disk: &'a mut D,
        record: Record7,
        bytes: &'a [u8],
    ) -> Self {
        volume.fenced = true;
        Self {
            volume,
            disk,
            candidate: None,
            bytes: Some(bytes),
            retry_record: Some(record),
            retry_matches: true,
            retry_checksum: !0u32,
            payload_offset: 0,
            step: Step::RetryRead(0),
            phase: Publication7Phase::Retrying,
            result: None,
            error: None,
            pending: false,
            stopping: false,
            io_started: false,
            block_prepared: false,
            block: [0; 512],
            nodes_checksum: !0u32,
            map_checksum: !0u32,
            receipts_checksum: !0u32,
            published_header: None,
        }
    }

    pub(super) fn replay(volume: &'a mut Volume7, disk: &'a mut D, record: Record7) -> Self {
        Self {
            volume,
            disk,
            candidate: None,
            bytes: None,
            retry_record: None,
            retry_matches: true,
            retry_checksum: !0u32,
            payload_offset: 0,
            step: Step::Done,
            phase: Publication7Phase::Committed,
            result: Some(record),
            error: None,
            pending: false,
            stopping: false,
            io_started: false,
            block_prepared: false,
            block: [0; 512],
            nodes_checksum: !0u32,
            map_checksum: !0u32,
            receipts_checksum: !0u32,
            published_header: None,
        }
    }

    pub fn phase(&self) -> Publication7Phase {
        self.phase
    }

    /// The durable record is hidden until the exact retry or header flush settles.
    pub fn result(&self) -> Option<Record7> {
        (self.phase == Publication7Phase::Committed)
            .then_some(self.result)
            .flatten()
    }

    pub fn pending(&self) -> bool {
        self.pending
    }

    /// Stop before header submission. An outstanding command must settle first;
    /// once header submission begins, cancellation is too late and settlement
    /// continues to determine the mounted head.
    pub fn abort_before_header(&mut self) -> Result<Publication7Cancel, Error> {
        match self.phase {
            Publication7Phase::Settling | Publication7Phase::Committed => {
                Ok(Publication7Cancel::TooLate)
            }
            Publication7Phase::Uncertain | Publication7Phase::Failed => {
                Err(self.error.unwrap_or(Error::Uncertain))
            }
            Publication7Phase::Cancelled => Ok(Publication7Cancel::Cancelled),
            Publication7Phase::Retrying
            | Publication7Phase::Preparing
            | Publication7Phase::ReadyToPublish => {
                if self.pending {
                    self.stopping = true;
                    return Ok(Publication7Cancel::Draining);
                }
                self.phase = Publication7Phase::Cancelled;
                self.volume.fenced = false;
                Ok(Publication7Cancel::Cancelled)
            }
        }
    }

    fn retry_sector_count(record: &Record7) -> u64 {
        record.runs().iter().map(|run| run.sectors).sum()
    }

    fn retry_sector(record: &Record7, mut index: u64) -> Option<u64> {
        for run in record.runs() {
            if index < run.sectors {
                return Some(run.start + index);
            }
            index -= run.sectors;
        }
        None
    }

    fn added_sectors(&self) -> u64 {
        self.candidate.as_ref().map_or(0, Candidate7::added_sectors)
    }

    fn prepare_block(&mut self) -> Result<(), Error> {
        if self.block_prepared {
            return Ok(());
        }
        match self.step {
            Step::PayloadWrite(index) => {
                let candidate = self.candidate.as_ref().ok_or(Error::Corrupt)?;
                let _run_sector = plan_sector(&candidate.added, index).ok_or(Error::Corrupt)?;
                let bytes = self.bytes.ok_or(Error::Corrupt)?;
                let offset = usize::try_from(index)
                    .ok()
                    .and_then(|sector| sector.checked_mul(512))
                    .ok_or(Error::Exhausted)?;
                self.block.fill(0);
                let count = bytes.len().saturating_sub(offset).min(self.block.len());
                self.block[..count].copy_from_slice(&bytes[offset..offset + count]);
            }
            Step::NodesWrite(index) => {
                self.encode_nodes(index)?;
                crc_update(&mut self.nodes_checksum, &self.block);
            }
            Step::MapWrite(index) => {
                self.encode_map(index)?;
                crc_update(&mut self.map_checksum, &self.block);
            }
            Step::ReceiptsWrite(index) => {
                self.encode_receipts(index)?;
                crc_update(&mut self.receipts_checksum, &self.block);
            }
            Step::HeaderWrite => {
                let header = self.published_header.ok_or(Error::Corrupt)?;
                self.block = header.encode()?;
            }
            Step::RetryRead(_) | Step::PayloadFlush | Step::MetadataFlush | Step::HeaderFlush => {}
            Step::Done => return Err(Error::Invalid),
        }
        self.block_prepared = true;
        Ok(())
    }

    fn encode_nodes(&mut self, sector: u64) -> Result<(), Error> {
        let candidate = self.candidate.as_ref().ok_or(Error::Corrupt)?;
        for offset in 0..512 / NODE_BYTES {
            let index = sector as usize * (512 / NODE_BYTES) + offset;
            let node = candidate
                .node_update
                .filter(|(changed, _)| *changed == index)
                .map_or(self.volume.nodes[index], |(_, changed)| changed);
            let encoded = node.encode()?;
            let at = offset * NODE_BYTES;
            self.block[at..at + NODE_BYTES].copy_from_slice(&encoded);
        }
        Ok(())
    }

    fn encode_map(&mut self, sector: u64) -> Result<(), Error> {
        let candidate = self.candidate.as_ref().ok_or(Error::Corrupt)?;
        for offset in 0..512 / 8 {
            let index = sector as usize * (512 / 8) + offset;
            let mut word = self.volume.map[index];
            mutate_word(&mut word, index, candidate.added.runs(), true);
            mutate_word(&mut word, index, candidate.release_runs(), false);
            self.block[offset * 8..offset * 8 + 8].copy_from_slice(&word.to_le_bytes());
        }
        Ok(())
    }

    fn encode_receipts(&mut self, sector: u64) -> Result<(), Error> {
        let candidate = self.candidate.as_ref().ok_or(Error::Corrupt)?;
        self.block.fill(0);
        let block_start = sector as usize * 512;
        let block_end = block_start + 512;
        for index in 0..RETAINED {
            let record = if index == candidate.record_slot {
                Some(candidate.record)
            } else {
                self.volume.records[index]
            };
            let Some(record) = record else {
                continue;
            };
            let record_start = index * RECORD_BYTES;
            let record_end = record_start + RECORD_BYTES;
            let copy_start = block_start.max(record_start);
            let copy_end = block_end.min(record_end);
            if copy_start >= copy_end {
                continue;
            }
            let encoded = record.encode()?;
            let source = copy_start - record_start;
            let destination = copy_start - block_start;
            let count = copy_end - copy_start;
            self.block[destination..destination + count]
                .copy_from_slice(&encoded[source..source + count]);
        }
        Ok(())
    }

    fn poll_command(&mut self) -> Poll<Result<(), Error>>
    where
        D: PollDisk7,
    {
        match self.step {
            Step::RetryRead(index) => {
                let record = self.retry_record.expect("retry record");
                let sector = Self::retry_sector(&record, index).expect("retry sector");
                self.disk
                    .poll_read(crate::format7::PAYLOAD_SECTOR + sector, &mut self.block)
            }
            Step::PayloadWrite(index) => {
                let candidate = self.candidate.as_ref().expect("candidate");
                let sector = plan_sector(&candidate.added, index).expect("payload sector");
                self.disk
                    .poll_write(crate::format7::PAYLOAD_SECTOR + sector, &self.block)
            }
            Step::PayloadFlush | Step::MetadataFlush | Step::HeaderFlush => self.disk.poll_flush(),
            Step::NodesWrite(index) => self.disk.poll_write(
                nodes_sector(self.volume.header.generation ^ 1) + index,
                &self.block,
            ),
            Step::MapWrite(index) => self.disk.poll_write(
                map_sector(self.volume.header.generation ^ 1) + index,
                &self.block,
            ),
            Step::ReceiptsWrite(index) => self.disk.poll_write(
                receipts_sector(self.volume.header.generation ^ 1) + index,
                &self.block,
            ),
            Step::HeaderWrite => self.disk.poll_write(
                header_sector(self.published_header.expect("candidate header").generation),
                &self.block,
            ),
            Step::Done => Poll::Ready(Ok(())),
        }
    }

    fn mark_failure(
        &mut self,
        error: Error,
        fence: bool,
    ) -> Poll<Result<Publication7Phase, Error>> {
        self.pending = false;
        self.phase = if fence {
            self.volume.fenced = true;
            Publication7Phase::Uncertain
        } else {
            self.volume.fenced = false;
            Publication7Phase::Failed
        };
        self.error = Some(error);
        Poll::Ready(Err(error))
    }

    fn finish_retry(&mut self) -> Poll<Result<Publication7Phase, Error>> {
        let record = self.retry_record.expect("retry record");
        if !self.retry_checksum != record.payload_crc32 {
            return self.mark_failure(Error::Corrupt, true);
        }
        if !self.retry_matches {
            return self.mark_failure(Error::IdempotencyConflict, false);
        }
        self.result = Some(record);
        self.phase = Publication7Phase::Committed;
        self.volume.fenced = false;
        Poll::Ready(Ok(self.phase))
    }

    fn complete_command(&mut self) -> Poll<Result<Publication7Phase, Error>> {
        self.pending = false;
        self.block_prepared = false;
        if self.stopping {
            self.phase = Publication7Phase::Cancelled;
            self.volume.fenced = false;
            return Poll::Ready(Ok(self.phase));
        }

        match self.step {
            Step::RetryRead(index) => {
                let bytes = self.bytes.expect("retry bytes");
                let count = (bytes.len() - self.payload_offset).min(self.block.len());
                crc_update(&mut self.retry_checksum, &self.block[..count]);
                if self.block[..count] != bytes[self.payload_offset..self.payload_offset + count] {
                    self.retry_matches = false;
                }
                self.payload_offset += count;
                let record = self.retry_record.expect("retry record");
                if index + 1 == Self::retry_sector_count(&record) {
                    return self.finish_retry();
                }
                self.step = Step::RetryRead(index + 1);
                self.phase = Publication7Phase::Retrying;
                Poll::Ready(Ok(self.phase))
            }
            Step::PayloadWrite(index) => {
                if index + 1 == self.added_sectors() {
                    self.step = Step::PayloadFlush;
                } else {
                    self.step = Step::PayloadWrite(index + 1);
                }
                self.phase = Publication7Phase::Preparing;
                Poll::Ready(Ok(self.phase))
            }
            Step::PayloadFlush => {
                self.step = Step::NodesWrite(0);
                self.phase = Publication7Phase::Preparing;
                Poll::Ready(Ok(self.phase))
            }
            Step::NodesWrite(index) => {
                self.step = if index + 1 == NODES_SECTORS {
                    Step::MapWrite(0)
                } else {
                    Step::NodesWrite(index + 1)
                };
                self.phase = Publication7Phase::Preparing;
                Poll::Ready(Ok(self.phase))
            }
            Step::MapWrite(index) => {
                self.step = if index + 1 == MAP_SECTORS {
                    Step::ReceiptsWrite(0)
                } else {
                    Step::MapWrite(index + 1)
                };
                self.phase = Publication7Phase::Preparing;
                Poll::Ready(Ok(self.phase))
            }
            Step::ReceiptsWrite(index) => {
                self.step = if index + 1 == RECEIPTS_SECTORS {
                    Step::MetadataFlush
                } else {
                    Step::ReceiptsWrite(index + 1)
                };
                self.phase = Publication7Phase::Preparing;
                Poll::Ready(Ok(self.phase))
            }
            Step::MetadataFlush => {
                let header = self.volume.header;
                self.published_header = Some(Header7 {
                    sequence: header.sequence.checked_add(1).expect("preflight sequence"),
                    generation: header.generation ^ 1,
                    nodes_checksum: !self.nodes_checksum,
                    map_checksum: !self.map_checksum,
                    receipts_checksum: !self.receipts_checksum,
                    ..header
                });
                self.step = Step::HeaderWrite;
                self.phase = Publication7Phase::ReadyToPublish;
                Poll::Ready(Ok(self.phase))
            }
            Step::HeaderWrite => {
                self.step = Step::HeaderFlush;
                self.phase = Publication7Phase::Settling;
                Poll::Ready(Ok(self.phase))
            }
            Step::HeaderFlush => {
                let candidate = self.candidate.expect("candidate");
                let header = self.published_header.expect("candidate header");
                adopt_candidate(self.volume, candidate, header);
                self.result = Some(candidate.record);
                self.phase = Publication7Phase::Committed;
                self.volume.fenced = false;
                self.step = Step::Done;
                Poll::Ready(Ok(self.phase))
            }
            Step::Done => Poll::Ready(Ok(self.phase)),
        }
    }

    fn stage_phase(&self) -> Publication7Phase {
        match self.step {
            Step::RetryRead(_) => Publication7Phase::Retrying,
            Step::HeaderWrite | Step::HeaderFlush => Publication7Phase::Settling,
            Step::Done => self.phase,
            _ => Publication7Phase::Preparing,
        }
    }

    fn prepare_retry_read(&mut self) -> Poll<Result<Publication7Phase, Error>> {
        let record = self.retry_record.expect("retry record");
        if Self::retry_sector_count(&record) == 0 {
            return self.finish_retry();
        }
        Poll::Pending
    }
}

impl<D: PollDisk7> PollPublication7<'_, D> {
    pub fn poll_advance(&mut self) -> Poll<Result<Publication7Phase, Error>> {
        match self.phase {
            Publication7Phase::Committed | Publication7Phase::Cancelled => {
                return Poll::Ready(Ok(self.phase));
            }
            Publication7Phase::Failed | Publication7Phase::Uncertain => {
                return Poll::Ready(Err(self.error.unwrap_or(Error::Uncertain)));
            }
            _ => (),
        }
        if matches!(self.step, Step::RetryRead(_))
            && Self::retry_sector_count(&self.retry_record.expect("retry record")) == 0
        {
            return self.prepare_retry_read();
        }
        if let Err(error) = self.prepare_block() {
            return self.mark_failure(error, self.io_started);
        }

        self.pending = true;
        self.io_started = true;
        self.phase = Publication7Phase::Uncertain;
        match self.poll_command() {
            Poll::Pending => {
                self.phase = self.stage_phase();
                Poll::Pending
            }
            Poll::Ready(Err(_)) => self.mark_failure(Error::Uncertain, true),
            Poll::Ready(Ok(())) => self.complete_command(),
        }
    }
}

impl<D> Drop for PollPublication7<'_, D> {
    fn drop(&mut self) {
        if self.pending || self.phase == Publication7Phase::Settling {
            self.volume.fenced = true;
        } else if matches!(
            self.phase,
            Publication7Phase::Preparing
                | Publication7Phase::ReadyToPublish
                | Publication7Phase::Retrying
        ) {
            // Before header submission all writes name only scratch bytes and
            // inactive metadata; the current in-memory generation stays intact.
            self.volume.fenced = false;
        }
    }
}

impl PayloadPlan {
    fn added_sectors(&self) -> u64 {
        self.runs().iter().map(|run| run.sectors).sum()
    }
}

impl Candidate7 {
    fn added_sectors(&self) -> u64 {
        self.added.added_sectors()
    }
}

fn plan_sector(plan: &PayloadPlan, mut index: u64) -> Option<u64> {
    for run in plan.runs() {
        if index < run.sectors {
            return Some(run.start + index);
        }
        index -= run.sectors;
    }
    None
}

fn mutate_word(word: &mut u64, word_index: usize, runs: &[Extent], allocated: bool) {
    let word_first = (word_index as u64) * 64;
    let word_end = word_first + 64;
    for run in runs {
        let first = run.start.max(word_first);
        let end = run.end().min(word_end);
        for sector in first..end {
            let bit = 1u64 << (sector % 64);
            if allocated {
                *word |= bit;
            } else {
                *word &= !bit;
            }
        }
    }
}

fn mutate_map(map: &mut [u64], runs: &[Extent], allocated: bool) {
    for run in runs {
        for sector in run.start..run.end() {
            let word = &mut map[sector as usize / 64];
            let bit = 1u64 << (sector % 64);
            if allocated {
                *word |= bit;
            } else {
                *word &= !bit;
            }
        }
    }
}

/// Validate a small metadata delta against the exact next header before any
/// candidate bytes or metadata reach disk.
pub(super) fn validate_candidate(volume: &mut Volume7, candidate: Candidate7) -> Result<(), Error> {
    candidate.record.encode()?;
    let header = Header7 {
        sequence: volume
            .header
            .sequence
            .checked_add(1)
            .ok_or(Error::Exhausted)?,
        generation: volume.header.generation ^ 1,
        ..volume.header
    };
    let old_record = volume.records[candidate.record_slot];
    let old_node = candidate
        .node_update
        .map(|(index, _)| (index, volume.nodes[index]));

    volume.records[candidate.record_slot] = Some(candidate.record);
    if let Some((index, node)) = candidate.node_update {
        volume.nodes[index] = node;
    }
    mutate_map(&mut volume.map, candidate.added.runs(), true);
    mutate_map(&mut volume.map, candidate.release_runs(), false);

    let result = validate_generation(
        &header,
        &volume.nodes,
        &volume.records,
        &volume.map,
        &mut volume.validation,
    );

    mutate_map(&mut volume.map, candidate.release_runs(), true);
    mutate_map(&mut volume.map, candidate.added.runs(), false);
    volume.records[candidate.record_slot] = old_record;
    if let Some((index, node)) = old_node {
        volume.nodes[index] = node;
    }
    result
}

fn adopt_candidate(volume: &mut Volume7, candidate: Candidate7, header: Header7) {
    volume.records[candidate.record_slot] = Some(candidate.record);
    if let Some((index, node)) = candidate.node_update {
        volume.nodes[index] = node;
    }
    mutate_map(&mut volume.map, candidate.added.runs(), true);
    mutate_map(&mut volume.map, candidate.release_runs(), false);
    volume.header = header;
}
