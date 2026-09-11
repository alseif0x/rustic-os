// SPDX-License-Identifier: Apache-2.0
//! Durable workspace namespace; authorization and hashing belong to the service.
use super::{Record, Retry};
use crate::{Disk, Error, MAX_FILE, Publication, Receipt, Volume};

#[derive(Clone, Copy, Debug)]
pub struct Replacement {
    pub workspace: u32,
    pub retry: Retry,
    pub id: u32,
    pub version: u64,
}
#[derive(Debug)]
pub struct Operation<'a> {
    pub workspace: u32,
    pub instance: u64,
    pub receipt: Receipt,
    pub bytes: &'a [u8],
}
impl Record {
    fn operation(&self) -> Result<Operation<'_>, Error> {
        let (workspace, instance) = self.namespace.ok_or(Error::OutcomeUnknown)?;
        if let Some(admission) = self.admission {
            match admission.state {
                crate::AdmissionState::Admitted => return Err(Error::Busy),
                crate::AdmissionState::Cancelled => return Err(Error::Cancelled),
                crate::AdmissionState::Committed => (),
            }
        }
        Ok(Operation {
            workspace,
            instance,
            receipt: self.receipt,
            bytes: &self.bytes[..usize::from(self.receipt.length)],
        })
    }
}
impl Volume {
    /// Explicit one-way publication of format v3. Legacy receipts retain their namespace.
    pub fn enable_operations(&mut self, disk: &mut impl Disk) -> Result<(), Error> {
        self.ready()?;
        let mut next = self.metadata.clone();
        let r = next.recovery.as_mut().ok_or(Error::Unsupported)?;
        if !r.scoped {
            r.scoped = true;
            self.commit(disk, next)?;
        }
        // A successful upgrade leaves no older-format fallback for an old kernel
        // to silently select. A cut still requires explicit recovery/retry.
        match crate::format::Metadata::read(disk, 1 - self.bank) {
            Ok(other) if other.recovery.as_ref().is_some_and(|r| r.scoped) => Ok(()),
            Ok(_) | Err(Error::Corrupt | Error::Empty) => self.commit(disk, self.metadata.clone()),
            Err(_) => {
                self.poisoned = true;
                Err(Error::Uncertain)
            }
        }
    }

    pub fn operations_enabled(&self) -> Result<bool, Error> {
        self.ready()?;
        Ok(self.metadata.recovery.as_ref().is_some_and(|r| r.scoped))
    }
    pub fn operation_by_retry(
        &self,
        subject: u64,
        workspace: u32,
        retry: Retry,
    ) -> Result<Operation<'_>, Error> {
        self.ready()?;
        let r = self
            .metadata
            .recovery
            .as_ref()
            .filter(|r| r.scoped)
            .ok_or(Error::Unsupported)?;
        if subject == 0 || workspace == 0 || retry.epoch == 0 || retry.key == 0 {
            return Err(Error::Invalid);
        }
        if retry.lineage != r.lineage {
            return Err(Error::Lineage);
        }
        if let Some(record) = r.records.iter().flatten().find(|record| {
            record.namespace.is_some_and(|(w, _)| w == workspace)
                && record.subject == subject
                && record.receipt.retry == retry
        }) {
            return record.operation();
        }
        if retry.epoch != r.epoch {
            return Err(Error::ExpiredEpoch);
        }
        Err(Error::OutcomeUnknown)
    }
    pub fn operation_by_id(
        &self,
        subject: u64,
        lineage: [u8; 16],
        committed: u64,
    ) -> Result<Operation<'_>, Error> {
        self.ready()?;
        let r = self
            .metadata
            .recovery
            .as_ref()
            .filter(|r| r.scoped)
            .ok_or(Error::Unsupported)?;
        if subject == 0 || committed == 0 {
            return Err(Error::Invalid);
        }
        if lineage != r.lineage {
            return Err(Error::Lineage);
        }
        r.records
            .iter()
            .flatten()
            .find(|record| record.subject == subject && record.receipt.committed == committed)
            .ok_or(Error::OutcomeUnknown)?
            .operation()
    }
    /// Assign the first committed sequence to a service incarnation lazily. The
    /// service retains it for later new operations; replay never assigns an incarnation.
    /// A failed commit poisons the volume and cannot resume this live writer.
    pub fn replace_scoped(
        &mut self,
        disk: &mut impl Disk,
        subject: u64,
        instance: u64,
        request: Replacement,
        bytes: &[u8],
    ) -> Result<Receipt, Error> {
        self.prepare_scoped(disk, subject, instance, request, bytes)?
            .run()
    }

    /// Volatile preparation or an already committed replay. No queued/running
    /// acceptance or durable cancellation is introduced by this mechanics API.
    pub fn prepare_scoped<'a, D>(
        &'a mut self,
        disk: &'a mut D,
        subject: u64,
        instance: u64,
        request: Replacement,
        bytes: &[u8],
    ) -> Result<Publication<'a, D, Receipt>, Error> {
        self.ready()?;
        if bytes.len() > MAX_FILE {
            return Err(Error::Size);
        }
        match self.operation_by_retry(subject, request.workspace, request.retry) {
            Ok(old) => {
                if old.receipt.id != request.id
                    || old.receipt.previous != request.version
                    || old.bytes != bytes
                {
                    return Err(Error::IdempotencyConflict);
                }
                let receipt = old.receipt;
                return Ok(Publication::replayed(self, disk, receipt));
            }
            Err(Error::OutcomeUnknown) => (),
            Err(error) => return Err(error),
        }
        self.resolve(request.workspace, request.id)?;
        let r = self.metadata.recovery.as_ref().unwrap();
        let slot = r
            .records
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Full)?;
        let committed = self
            .metadata
            .sequence
            .checked_add(1)
            .ok_or(Error::Exhausted)?;
        let instance = if instance == 0 { committed } else { instance };
        if instance > committed {
            return Err(Error::Invalid);
        }
        let receipt = Receipt {
            retry: request.retry,
            id: request.id,
            previous: request.version,
            committed,
            length: bytes.len() as u16,
        };
        let mut record = Record {
            subject,
            receipt,
            namespace: Some((request.workspace, instance)),
            bytes: [0; MAX_FILE],
            admission: None,
        };
        record.bytes[..bytes.len()].copy_from_slice(bytes);
        let mut next = self.metadata.clone();
        next.recovery.as_mut().unwrap().records[slot] = Some(record);
        self.prepare_recorded(disk, request.id, request.version, bytes, next, |_| receipt)
    }
}
