// SPDX-License-Identifier: Apache-2.0
//! Preflight, exact retry handling and candidate construction for v7 replacement.

use crate::extent::Extent;
use crate::format7::{NEXT_MIN, Node7, Record7, RecordState};
use crate::{Disk, Error, Kind};

use super::payload::{PayloadPlan, plan_payload, release_run, reserve_plan};
use super::poll::{Candidate7, validate_candidate};
use super::stage::{Opening, Stage7, Stage7Kind, StageMode, StageSlot};
use super::{Volume7, WriteIdentity7};

/// The live file and fresh resources a tracked replacement would publish.
struct TrackedTarget {
    index: usize,
    previous: Node7,
    receipt_slot: usize,
    sequence: u64,
}

impl Volume7 {
    /// Replace one file and retain a `DirectCommitted` retry record with the
    /// exact bytes produced by the operation.
    ///
    /// An exact retained retry reads and verifies its immutable snapshot but
    /// performs no writes or flushes. A new operation allocates only sectors the
    /// selected generation currently marks free, then atomically publishes the
    /// candidate through the inactive generation's header.
    ///
    /// This is the borrowed-bytes form of [`Volume7::open_stage`] with
    /// [`Stage7Kind::Tracked`], one [`Volume7::stage_write`] per sector and
    /// [`Volume7::finish_tracked`]; it uses no stage slot of its own.
    pub fn replace_tracked(
        &mut self,
        disk: &mut impl Disk,
        identity: WriteIdentity7,
        expected_version: u64,
        bytes: &[u8],
    ) -> Result<Record7, Error> {
        self.ready()?;
        let length = u32::try_from(bytes.len()).map_err(|_| Error::Size)?;
        let mut slot = self.open_slot(identity, expected_version, length, Stage7Kind::Tracked)?;
        for sector in bytes.chunks(512) {
            self.write_slot(disk, &mut slot, sector)
                .map_err(|fault| fault.error())?;
        }
        self.finish_tracked_slot(disk, slot)
    }

    /// Finish a fully written tracked stage.
    ///
    /// A stage of another kind or with sectors still expected is refused with
    /// [`Error::Invalid`] and stays open. A verifying retry returns the
    /// retained record without writes or flushes; a snapshot CRC mismatch
    /// returns [`Error::Corrupt`] before a byte difference returns
    /// [`Error::IdempotencyConflict`]. A fresh stage rechecks the file version,
    /// epoch, retry scope and receipt slot and validates the candidate before
    /// any metadata write; a refusal releases the stage without fencing.
    /// Publication failures return [`Error::Uncertain`] and fence the owner.
    /// Apart from the `Invalid` refusal above, the stage ends and the token is
    /// stale afterwards.
    pub fn finish_tracked(
        &mut self,
        disk: &mut impl Disk,
        stage: &mut Stage7,
    ) -> Result<Record7, Error> {
        self.ready()?;
        let slot = self.take_finishable(stage, Stage7Kind::Tracked)?;
        self.finish_tracked_slot(disk, slot)
    }

    fn finish_tracked_slot(
        &mut self,
        disk: &mut impl Disk,
        slot: StageSlot,
    ) -> Result<Record7, Error> {
        if slot.kind != Stage7Kind::Tracked || !slot.complete() {
            return Err(Error::Invalid);
        }
        match slot.mode {
            StageMode::Retry { record, matches } => {
                if slot.payload_crc() != record.payload_crc32 {
                    return Err(Error::Corrupt);
                }
                if !matches {
                    return Err(Error::IdempotencyConflict);
                }
                Ok(record)
            }
            StageMode::Fresh(plan) => self.commit_tracked(disk, &slot, plan),
        }
    }

    /// Tracked preflight: exact retry rules, then the fresh target and a plan
    /// that avoids every open stage's reservation.
    pub(super) fn open_tracked(
        &mut self,
        identity: WriteIdentity7,
        expected_version: u64,
        length: u32,
    ) -> Result<Opening, Error> {
        if let Some(record) = self.find_retry(identity) {
            if record.object != identity.object
                || record.instance != identity.instance
                || record.previous != expected_version
                || record.length != length
            {
                return Err(Error::IdempotencyConflict);
            }
            if record.state != RecordState::DirectCommitted {
                return Err(Error::OutcomeUnknown);
            }
            return Ok(Opening::Retry(*record));
        }
        self.tracked_target(identity, expected_version)?;
        let plan = plan_payload(self.planning_map(), length as usize)?;
        Ok(Opening::Fresh(plan))
    }

    fn tracked_target(
        &self,
        identity: WriteIdentity7,
        expected_version: u64,
    ) -> Result<TrackedTarget, Error> {
        if identity.retry_epoch != self.header.epoch {
            return Err(Error::ExpiredEpoch);
        }
        let index = self
            .nodes
            .iter()
            .position(|node| node.kind != Kind::Empty && node.id == identity.object)
            .ok_or(Error::NotFound)?;
        let previous = self.nodes[index];
        if previous.kind != Kind::File {
            return Err(Error::IsDirectory);
        }
        if previous.version != expected_version {
            return Err(Error::Version);
        }
        let receipt_slot = self.free_record_slot()?;
        let sequence = self
            .header
            .sequence
            .checked_add(1)
            .ok_or(Error::Exhausted)?;
        let minimum_version = expected_version.checked_add(1).ok_or(Error::Exhausted)?;
        if sequence < minimum_version {
            return Err(Error::Corrupt);
        }
        if identity.instance > sequence {
            return Err(Error::Invalid);
        }
        Ok(TrackedTarget {
            index,
            previous,
            receipt_slot,
            sequence,
        })
    }

    /// Recheck, validate and publish a fresh tracked stage whose payload is
    /// already on disk in `plan`.
    fn commit_tracked(
        &mut self,
        disk: &mut impl Disk,
        slot: &StageSlot,
        plan: PayloadPlan,
    ) -> Result<Record7, Error> {
        let identity = slot.identity;
        if self.find_retry(identity).is_some() {
            return Err(Error::IdempotencyConflict);
        }
        let target = self.tracked_target(identity, slot.expected_version)?;
        let record = Record7 {
            subject: identity.subject,
            workspace: identity.workspace,
            object: identity.object,
            instance: identity.instance,
            retry_epoch: identity.retry_epoch,
            retry_key: identity.retry_key,
            previous: slot.expected_version,
            committed: target.sequence,
            admission_number: 0,
            terminal: target.sequence,
            length: slot.length,
            payload_crc32: slot.payload_crc(),
            state: RecordState::DirectCommitted,
            prevention: None,
            extents_used: plan.used as u8,
            extents: plan.runs,
        };
        let mut next_node = target.previous;
        next_node.version = target.sequence;
        next_node.length = slot.length;
        next_node.payload_crc32 = record.payload_crc32;
        next_node.extents_used = plan.used as u8;
        next_node.extents = plan.runs;

        let mut candidate = Candidate7::empty(target.receipt_slot, record);
        candidate.node_update = Some((target.index, next_node));
        candidate.added = plan;
        for run in target.previous.runs() {
            if !retained_owns_run(&self.records, *run) {
                candidate.released[candidate.released_used] = *run;
                candidate.released_used += 1;
            }
        }
        validate_candidate(self, candidate)?;

        self.fenced = true;
        reserve_plan(&mut self.map, &plan);
        for run in &candidate.released[..candidate.released_used] {
            if release_run(&mut self.map, *run).is_err() {
                self.fence_clear();
                return Err(Error::Uncertain);
            }
        }
        self.nodes[target.index] = next_node;
        self.records[target.receipt_slot] = Some(record);

        match self.publish_candidate(disk) {
            Ok(header) => {
                self.header = header;
                self.fenced = false;
                Ok(record)
            }
            Err(error) => {
                self.fence_clear();
                Err(error)
            }
        }
    }

    pub(super) fn validate_identity(&self, identity: WriteIdentity7) -> Result<(), Error> {
        if identity.subject == 0
            || identity.workspace == 0
            || identity.object < NEXT_MIN
            || identity.instance == 0
            || identity.retry_epoch == 0
            || identity.retry_key == 0
            || identity.workspace >= self.header.next
            || identity.object >= self.header.next
            || identity.workspace == identity.object
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }

    pub(super) fn find_retry(&self, identity: WriteIdentity7) -> Option<&Record7> {
        self.records.iter().flatten().find(|record| {
            record.subject == identity.subject
                && record.workspace == identity.workspace
                && record.retry_epoch == identity.retry_epoch
                && record.retry_key == identity.retry_key
        })
    }

    pub(super) fn fence_clear(&mut self) {
        self.clear();
        self.fenced = true;
    }
}

pub(super) fn retained_owns_run(records: &[Option<Record7>], run: Extent) -> bool {
    records.iter().flatten().any(|record| {
        record
            .runs()
            .iter()
            .any(|owned| run.start < owned.end() && owned.start < run.end())
    })
}
