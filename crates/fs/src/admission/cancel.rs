// SPDX-License-Identifier: Apache-2.0
use super::{AdmissionId, AdmissionState, AdmissionStatus, PreventionReason};
use crate::{Disk, Error, Publication, Volume};
impl Volume {
    /// Persist a no-effect terminal state only while no publication is in flight.
    /// Returns the immutable existing terminal state if already settled. A caller
    /// must examine it: Committed means cancellation was too late, not rollback.
    pub fn cancel_admission(
        &mut self,
        disk: &mut impl Disk,
        subject: u64,
        id: AdmissionId,
    ) -> Result<AdmissionStatus, Error> {
        self.prepare_cancellation(disk, subject, id)?.run()
    }

    /// Prepare a terminal cancellation without I/O. Cancelling this metadata
    /// guard leaves the record Admitted; only its result proves durable cancellation.
    #[inline(never)]
    pub fn prepare_cancellation<'a, D>(
        &'a mut self,
        disk: &'a mut D,
        subject: u64,
        id: AdmissionId,
    ) -> Result<Publication<'a, D, AdmissionStatus>, Error> {
        let reason = if self.prevention_reasons_enabled()? {
            PreventionReason::Requested
        } else {
            PreventionReason::Unknown
        };
        self.prepare_cancelled(disk, subject, id, reason)
    }

    /// Persist a service-selected cause on an explicitly migrated v5 volume.
    /// Older volumes return Unsupported; a terminal replay never changes its cause.
    #[inline(never)]
    pub fn prepare_prevention<'a, D>(
        &'a mut self,
        disk: &'a mut D,
        subject: u64,
        id: AdmissionId,
        reason: PreventionReason,
    ) -> Result<Publication<'a, D, AdmissionStatus>, Error> {
        if !self.prevention_reasons_enabled()? {
            return Err(Error::Unsupported);
        }
        self.prepare_cancelled(disk, subject, id, reason)
    }

    fn prepare_cancelled<'a, D>(
        &'a mut self,
        disk: &'a mut D,
        subject: u64,
        id: AdmissionId,
        reason: PreventionReason,
    ) -> Result<Publication<'a, D, AdmissionStatus>, Error> {
        let slot = self.admission_slot(subject, id)?;
        let old = self.admission_by_id(subject, id)?.status;
        if old.state != AdmissionState::Admitted {
            return Ok(Publication::replayed(self, disk, old));
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
        a.prevention = Some(reason);
        Publication::metadata(
            self,
            disk,
            next,
            AdmissionStatus {
                state: AdmissionState::Cancelled,
                terminal,
                prevention: Some(reason),
                ..old
            },
        )
    }
}
