// SPDX-License-Identifier: Apache-2.0
use super::{AdmissionId, AdmissionState};
use crate::{Error, Publication, Receipt, Volume};
impl Volume {
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
