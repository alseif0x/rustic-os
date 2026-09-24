// SPDX-License-Identifier: Apache-2.0
//! Durable staged admission (borrowed or streamed), explicit cancellation, and
//! admitted replacement.

use crate::extent::Extent;
use crate::format7::{MAX_EXTENTS, Record7, RecordState};
use crate::{Error, Kind, PreventionReason};

use super::payload::{PayloadPlan, payload_crc, plan_payload};
use super::poll::{Candidate7, PollDisk7, PollPublication7, validate_candidate};
use super::stage::{Opening, Stage7, Stage7Kind, StageMode};
use super::{Volume7, WriteIdentity7, replacement::retained_owns_run};

impl Volume7 {
    /// Begin a pollable durable admission for an existing file.
    ///
    /// Caller bytes stay borrowed until settlement. They are streamed to the
    /// planned free extents, flushed, and only then named by the admitted
    /// metadata and header. A retained exact retry is verified through bounded
    /// poll reads and does not publish a new generation.
    pub fn prepare_admission<'a, D: PollDisk7>(
        &'a mut self,
        disk: &'a mut D,
        identity: WriteIdentity7,
        expected_version: u64,
        bytes: &'a [u8],
    ) -> Result<PollPublication7<'a, D>, Error> {
        self.ready()?;
        if bytes.len() > crate::format7::MAX_FILE_BYTES as usize {
            return Err(Error::Size);
        }
        self.validate_identity(identity)?;
        if self.stage_holds_scope(identity) {
            return Err(Error::Busy);
        }
        let length = bytes.len() as u32;
        match self.open_admission(identity, expected_version, length)? {
            Opening::Retry(record) => Ok(PollPublication7::retry(self, disk, record, bytes)),
            Opening::Fresh(plan) => {
                let candidate = self.admission_candidate(
                    identity,
                    expected_version,
                    length,
                    payload_crc(bytes),
                    plan,
                )?;
                Ok(PollPublication7::publish(
                    self,
                    disk,
                    candidate,
                    Some(bytes),
                ))
            }
        }
    }

    /// Finish a fully written admission stage as a pollable publication.
    ///
    /// A fresh stage's payload is already written, so the publication starts
    /// at the payload flush and then follows the same barriers as
    /// [`Volume7::prepare_admission`]. The file version, epoch, retry scope and
    /// receipt slot are rechecked and the candidate validated first; a refusal
    /// releases the stage without I/O or fencing. A verifying retry returns a
    /// settled publication with the retained record; as in the poll retry path,
    /// a snapshot CRC mismatch returns [`Error::Corrupt`] and fences, and a
    /// byte difference returns [`Error::IdempotencyConflict`]. A stage of
    /// another kind or with sectors still expected is refused with
    /// [`Error::Invalid`] and stays open; otherwise the stage ends and the
    /// token is stale afterwards.
    pub fn finish_admission<'a, D: PollDisk7>(
        &'a mut self,
        disk: &'a mut D,
        stage: &mut Stage7,
    ) -> Result<PollPublication7<'a, D>, Error> {
        self.ready()?;
        let slot = self.take_finishable(stage, Stage7Kind::Admission)?;
        match slot.mode {
            StageMode::Retry { record, matches } => {
                if slot.payload_crc() != record.payload_crc32 {
                    self.fenced = true;
                    return Err(Error::Corrupt);
                }
                if !matches {
                    return Err(Error::IdempotencyConflict);
                }
                // Execution or cancellation may have advanced the retained
                // record while the stage was open; report its current state.
                let current = self
                    .find_retry(slot.identity)
                    .copied()
                    .ok_or(Error::Corrupt)?;
                Ok(PollPublication7::replay(self, disk, current))
            }
            StageMode::Fresh(plan) => {
                if self.find_retry(slot.identity).is_some() {
                    return Err(Error::IdempotencyConflict);
                }
                let candidate = self.admission_candidate(
                    slot.identity,
                    slot.expected_version,
                    slot.length,
                    slot.payload_crc(),
                    plan,
                )?;
                Ok(PollPublication7::staged(self, disk, candidate))
            }
        }
    }

    /// Admission preflight: exact retry rules, then the fresh target and a plan
    /// that avoids every open stage's reservation.
    pub(super) fn open_admission(
        &mut self,
        identity: WriteIdentity7,
        expected_version: u64,
        length: u32,
    ) -> Result<Opening, Error> {
        if let Some(record) = self.find_retry(identity).copied() {
            if !same_request(&record, identity, expected_version, length as usize) {
                return Err(Error::IdempotencyConflict);
            }
            return Ok(Opening::Retry(record));
        }
        self.admission_target(identity, expected_version)?;
        let plan = plan_payload(self.planning_map(), length as usize)?;
        Ok(Opening::Fresh(plan))
    }

    /// The receipt slot and admission number a fresh admission would use.
    fn admission_target(
        &self,
        identity: WriteIdentity7,
        expected_version: u64,
    ) -> Result<(usize, u64), Error> {
        if identity.retry_epoch != self.header.epoch {
            return Err(Error::ExpiredEpoch);
        }
        let node_index = self
            .nodes
            .iter()
            .position(|node| node.kind != Kind::Empty && node.id == identity.object)
            .ok_or(Error::NotFound)?;
        let node = self.nodes[node_index];
        if node.kind != Kind::File {
            return Err(Error::IsDirectory);
        }
        if node.version != expected_version {
            return Err(Error::Version);
        }
        let record_slot = self.free_record_slot()?;
        let admission_number = self
            .header
            .sequence
            .checked_add(1)
            .ok_or(Error::Exhausted)?;
        if identity.instance > admission_number {
            return Err(Error::Invalid);
        }
        Ok((record_slot, admission_number))
    }

    /// Build and validate the admitted candidate for payload already planned.
    fn admission_candidate(
        &mut self,
        identity: WriteIdentity7,
        expected_version: u64,
        length: u32,
        payload_crc32: u32,
        plan: PayloadPlan,
    ) -> Result<Candidate7, Error> {
        let (record_slot, admission_number) = self.admission_target(identity, expected_version)?;
        let mut candidate_record = Record7 {
            subject: identity.subject,
            workspace: identity.workspace,
            object: identity.object,
            instance: identity.instance,
            retry_epoch: identity.retry_epoch,
            retry_key: identity.retry_key,
            previous: expected_version,
            committed: 0,
            admission_number,
            terminal: 0,
            length,
            payload_crc32,
            state: RecordState::Admitted,
            prevention: None,
            extents_used: plan.used as u8,
            extents: plan.runs,
        };
        // Keep the record snapshot canonical even for an empty file.
        if plan.used == 0 {
            candidate_record.extents = [Extent::new(0, 0); MAX_EXTENTS];
        }
        let mut candidate = Candidate7::empty(record_slot, candidate_record);
        candidate.added = plan;
        validate_candidate(self, candidate)?;
        Ok(candidate)
    }

    /// Prepare explicit execution of one durable admission.
    ///
    /// The current file identity and version are checked again before any disk
    /// command is submitted. A version conflict leaves the admission and its
    /// candidate snapshot intact so the service can durably cancel it instead.
    pub fn prepare_execute<'a, D: PollDisk7>(
        &'a mut self,
        disk: &'a mut D,
        identity: WriteIdentity7,
        expected_version: u64,
    ) -> Result<PollPublication7<'a, D>, Error> {
        self.ready()?;
        self.validate_identity(identity)?;
        let (record_slot, record) = self.lookup_record(identity)?;
        if !same_record_identity(record, identity, expected_version) {
            return Err(Error::IdempotencyConflict);
        }
        match record.state {
            RecordState::Cancelled => return Err(Error::Cancelled),
            RecordState::DirectCommitted => return Err(Error::IdempotencyConflict),
            RecordState::AdmittedCommitted => {
                return Ok(PollPublication7::replay(self, disk, record));
            }
            RecordState::Admitted => (),
        }

        let node_index = self
            .nodes
            .iter()
            .position(|node| node.kind != Kind::Empty && node.id == identity.object)
            .ok_or(Error::NotFound)?;
        let old_node = self.nodes[node_index];
        if old_node.kind != Kind::File {
            return Err(Error::IsDirectory);
        }
        if old_node.version != record.previous {
            return Err(Error::Version);
        }
        let committed = self
            .header
            .sequence
            .checked_add(1)
            .ok_or(Error::Exhausted)?;

        let mut committed_record = record;
        committed_record.committed = committed;
        committed_record.terminal = committed;
        committed_record.state = RecordState::AdmittedCommitted;
        let mut next_node = old_node;
        next_node.version = committed;
        next_node.length = record.length;
        next_node.payload_crc32 = record.payload_crc32;
        next_node.extents_used = record.extents_used;
        next_node.extents = record.extents;

        let mut candidate = Candidate7::empty(record_slot, committed_record);
        candidate.node_update = Some((node_index, next_node));
        for old_run in old_node.runs() {
            if !retained_owns_run(&self.records, *old_run) {
                candidate.released[candidate.released_used] = *old_run;
                candidate.released_used += 1;
            }
        }
        validate_candidate(self, candidate)?;
        Ok(PollPublication7::publish(self, disk, candidate, None))
    }

    /// Prepare a durable no-effect terminal cancellation with its supplied cause.
    ///
    /// The old live node and both payload ownership sets stay unchanged. An
    /// already-cancelled exact retry replays its cause; changing that cause is a
    /// conflict. A committed record is returned as-is so the caller can observe
    /// that cancellation was too late.
    pub fn prepare_cancellation<'a, D: PollDisk7>(
        &'a mut self,
        disk: &'a mut D,
        identity: WriteIdentity7,
        expected_version: u64,
        prevention: PreventionReason,
    ) -> Result<PollPublication7<'a, D>, Error> {
        self.ready()?;
        self.validate_identity(identity)?;
        let (record_slot, record) = self.lookup_record(identity)?;
        if !same_record_identity(record, identity, expected_version) {
            return Err(Error::IdempotencyConflict);
        }
        match record.state {
            RecordState::Cancelled => {
                if record.prevention != Some(prevention) {
                    return Err(Error::IdempotencyConflict);
                }
                return Ok(PollPublication7::replay(self, disk, record));
            }
            RecordState::AdmittedCommitted | RecordState::DirectCommitted => {
                return Ok(PollPublication7::replay(self, disk, record));
            }
            RecordState::Admitted => (),
        }
        let terminal = self
            .header
            .sequence
            .checked_add(1)
            .ok_or(Error::Exhausted)?;
        let mut cancelled = record;
        cancelled.committed = 0;
        cancelled.terminal = terminal;
        cancelled.state = RecordState::Cancelled;
        cancelled.prevention = Some(prevention);

        let candidate = Candidate7::empty(record_slot, cancelled);
        validate_candidate(self, candidate)?;
        Ok(PollPublication7::publish(self, disk, candidate, None))
    }

    fn lookup_record(&self, identity: WriteIdentity7) -> Result<(usize, Record7), Error> {
        if let Some((slot, record)) = self.records.iter().enumerate().find_map(|(slot, record)| {
            record
                .filter(|record| same_retry_scope(record, identity))
                .map(|record| (slot, record))
        }) {
            return Ok((slot, record));
        }
        if identity.retry_epoch != self.header.epoch {
            return Err(Error::ExpiredEpoch);
        }
        Err(Error::OutcomeUnknown)
    }
}

fn same_retry_scope(record: &Record7, identity: WriteIdentity7) -> bool {
    record.subject == identity.subject
        && record.workspace == identity.workspace
        && record.retry_epoch == identity.retry_epoch
        && record.retry_key == identity.retry_key
}

fn same_record_identity(record: Record7, identity: WriteIdentity7, expected_version: u64) -> bool {
    record.object == identity.object
        && record.instance == identity.instance
        && record.previous == expected_version
        && same_retry_scope(&record, identity)
}

fn same_request(
    record: &Record7,
    identity: WriteIdentity7,
    expected_version: u64,
    length: usize,
) -> bool {
    same_record_identity(*record, identity, expected_version) && record.length as usize == length
}
