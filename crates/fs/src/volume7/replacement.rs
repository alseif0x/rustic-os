// SPDX-License-Identifier: Apache-2.0
//! Preflight, exact retry handling and candidate construction for v7 replacement.

use crate::extent::Extent;
use crate::format7::{MAX_FILE_BYTES, NEXT_MIN, Record7, RecordState};
use crate::{Disk, Error, Kind};

use super::payload::{
    exact_retry, payload_crc, plan_payload, release_run, reserve_plan, write_payload,
};
use super::{Volume7, WriteIdentity7};

impl Volume7 {
    /// Replace one file and retain a `DirectCommitted` retry record with the
    /// exact bytes produced by the operation.
    ///
    /// An exact retained retry reads and verifies its immutable snapshot but
    /// performs no writes or flushes. A new operation allocates only sectors the
    /// selected generation currently marks free, then atomically publishes the
    /// candidate through the inactive generation's header.
    pub fn replace_tracked(
        &mut self,
        disk: &mut impl Disk,
        identity: WriteIdentity7,
        expected_version: u64,
        bytes: &[u8],
    ) -> Result<Record7, Error> {
        self.ready()?;
        if bytes.len() > MAX_FILE_BYTES as usize {
            return Err(Error::Size);
        }
        self.validate_identity(identity)?;

        if let Some(record) = self.find_retry(identity) {
            if record.object != identity.object
                || record.instance != identity.instance
                || record.previous != expected_version
                || record.length != bytes.len() as u32
            {
                return Err(Error::IdempotencyConflict);
            }
            if record.state != RecordState::DirectCommitted {
                return Err(Error::OutcomeUnknown);
            }
            if !exact_retry(disk, record, bytes)? {
                return Err(Error::IdempotencyConflict);
            }
            return Ok(*record);
        }
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
        let receipt_slot = self
            .records
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Full)?;
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
        let plan = plan_payload(&self.map, bytes.len())?;

        let record = Record7 {
            subject: identity.subject,
            workspace: identity.workspace,
            object: identity.object,
            instance: identity.instance,
            retry_epoch: identity.retry_epoch,
            retry_key: identity.retry_key,
            previous: expected_version,
            committed: sequence,
            admission_number: 0,
            terminal: sequence,
            length: bytes.len() as u32,
            payload_crc32: payload_crc(bytes),
            state: RecordState::DirectCommitted,
            prevention: None,
            extents_used: plan.used as u8,
            extents: plan.runs,
        };
        let mut next_node = previous;
        next_node.version = sequence;
        next_node.length = bytes.len() as u32;
        next_node.payload_crc32 = record.payload_crc32;
        next_node.extents_used = plan.used as u8;
        next_node.extents = plan.runs;

        self.fenced = true;
        reserve_plan(&mut self.map, &plan);
        if write_payload(disk, &plan, bytes).is_err() {
            self.fence_clear();
            return Err(Error::Uncertain);
        }

        for run in previous.runs() {
            if !retained_owns_run(&self.records, *run) && release_run(&mut self.map, *run).is_err()
            {
                self.fence_clear();
                return Err(Error::Uncertain);
            }
        }
        self.nodes[index] = next_node;
        self.records[receipt_slot] = Some(record);

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

    fn fence_clear(&mut self) {
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
