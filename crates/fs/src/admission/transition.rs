// SPDX-License-Identifier: Apache-2.0
use super::{AdmissionId, AdmissionState, AdmissionStatus, Stored};
use crate::{
    Disk, Error, Kind, MAX_FILE, Publication, Receipt, Replacement, Volume, recovery::Record,
};

impl Volume {
    /// Owner-requested one-way v4 migration; preserves all existing receipts.
    /// Complete both banks so an older reader cannot select a stale v3 fallback.
    pub fn enable_admissions(&mut self, disk: &mut impl Disk) -> Result<(), Error> {
        self.ready()?;
        let mut next = self.metadata.clone();
        let r = next
            .recovery
            .as_mut()
            .filter(|r| r.scoped)
            .ok_or(Error::Unsupported)?;
        if !r.admissions {
            r.admissions = true;
            self.commit(disk, next)?;
        }
        match crate::format::Metadata::read(disk, 1 - self.bank) {
            Ok(other) if other.recovery.as_ref().is_some_and(|r| r.admissions) => Ok(()),
            Ok(_) | Err(Error::Corrupt | Error::Empty) => self.commit(disk, self.metadata.clone()),
            Err(_) => {
                self.poisoned = true;
                Err(Error::Uncertain)
            }
        }
    }

    /// Reserves one of the existing two global slots and persists all arguments
    /// before returning an ID. Does not modify the file or retain authority.
    pub fn admit_replace(
        &mut self,
        disk: &mut impl Disk,
        subject: u64,
        instance: u64,
        request: Replacement,
        bytes: &[u8],
    ) -> Result<AdmissionStatus, Error> {
        if bytes.len() > MAX_FILE {
            return Err(Error::Size);
        }
        match self.admission_by_retry(subject, request.workspace, request.retry) {
            Ok(old) => {
                if old.request.id != request.id
                    || old.request.version != request.version
                    || old.bytes != bytes
                {
                    return Err(Error::IdempotencyConflict);
                }
                return Ok(old.status);
            }
            Err(Error::OutcomeUnknown) => (),
            Err(e) => return Err(e),
        }
        self.resolve(request.workspace, request.id)?;
        let node = self.writable(request.id)?;
        if node.kind != Kind::File {
            return Err(Error::IsDirectory);
        }
        if node.version != request.version {
            return Err(Error::Version);
        }
        let slot = self
            .admissions()?
            .records
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Full)?;
        let number = self.sequence().checked_add(1).ok_or(Error::Exhausted)?;
        let instance = if instance == 0 { number } else { instance };
        if instance > number {
            return Err(Error::Invalid);
        }
        let mut record = Record {
            subject,
            receipt: Receipt {
                retry: request.retry,
                id: request.id,
                previous: request.version,
                committed: 0,
                length: bytes.len() as u16,
            },
            namespace: Some((request.workspace, instance)),
            bytes: [0; MAX_FILE],
            admission: Some(Stored {
                number,
                state: AdmissionState::Admitted,
                terminal: 0,
            }),
        };
        record.bytes[..bytes.len()].copy_from_slice(bytes);
        let status = record.admission_view()?.status;
        let mut next = self.metadata.clone();
        next.recovery.as_mut().unwrap().records[slot] = Some(record);
        self.commit(disk, next)?;
        Ok(status)
    }

    /// Persist a no-effect terminal state only while no publication is in flight.
    /// Returns the immutable existing terminal state if already settled. A caller
    /// must examine it: Committed means cancellation was too late, not rollback.
    pub fn cancel_admission(
        &mut self,
        disk: &mut impl Disk,
        subject: u64,
        id: AdmissionId,
    ) -> Result<AdmissionStatus, Error> {
        let slot = self.admission_slot(subject, id)?;
        let old = self.admission_by_id(subject, id)?.status;
        if old.state != AdmissionState::Admitted {
            return Ok(old);
        }
        let terminal = self.sequence().checked_add(1).ok_or(Error::Exhausted)?;
        let mut next = self.metadata.clone();
        let a = next.recovery.as_mut().unwrap().records[slot]
            .as_mut()
            .unwrap()
            .admission
            .as_mut()
            .unwrap();
        a.state = AdmissionState::Cancelled;
        a.terminal = terminal;
        self.commit(disk, next)?;
        Ok(AdmissionStatus {
            state: AdmissionState::Cancelled,
            terminal,
            ..old
        })
    }

    /// Explicit execution after fresh authorization by the service. Rechecks
    /// the current file version and workspace. Early guard cancellation leaves
    /// Admitted until cancel_admission itself durably settles; never auto-resume.
    #[inline(never)]
    pub fn prepare_admitted<'a, D>(
        &'a mut self,
        disk: &'a mut D,
        subject: u64,
        id: AdmissionId,
    ) -> Result<Publication<'a, D, Receipt>, Error> {
        let slot = self.admission_slot(subject, id)?;
        let record = *self.admissions()?.records[slot].as_ref().unwrap();
        let admission = record.admission_view()?;
        match admission.status.state {
            AdmissionState::Cancelled => return Err(Error::Cancelled),
            AdmissionState::Committed => {
                return Ok(Publication::replayed(self, disk, record.receipt));
            }
            AdmissionState::Admitted => (),
        }
        self.resolve(admission.request.workspace, record.receipt.id)?;
        let mut next = self.metadata.clone();
        let r = next.recovery.as_mut().unwrap().records[slot]
            .as_mut()
            .unwrap();
        r.receipt.committed = self.sequence().checked_add(1).ok_or(Error::Exhausted)?;
        let a = r.admission.as_mut().unwrap();
        a.state = AdmissionState::Committed;
        a.terminal = r.receipt.committed;
        let receipt = r.receipt;
        self.prepare_recorded(
            disk,
            receipt.id,
            receipt.previous,
            admission.bytes,
            next,
            |_| receipt,
        )
    }
}
