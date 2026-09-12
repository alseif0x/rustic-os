// SPDX-License-Identifier: Apache-2.0
//! Service-owned durable prevention after an accepted stop or lost execution guard.
use super::{ActiveExecution, control::drive_stoppable};
use crate::{Clients, Server, reply};
use rustic_abi::files::Error;
use rustic_fs::{
    AdmissionId, AdmissionState, AdmissionStatus, PollDisk, PreventionReason, Publication, Volume,
};

/// Compatibility is explicit here: v4 retains prevention without a cause. It is
/// never upgraded by a service request. v5 must preserve the policy's exact cause.
pub(super) fn publication<'a, D>(
    volume: &'a mut Volume,
    disk: &'a mut D,
    subject: u64,
    id: AdmissionId,
    reason: PreventionReason,
) -> Result<Publication<'a, D, AdmissionStatus>, Error> {
    if volume.prevention_reasons_enabled().map_err(reply::error)? {
        volume.prepare_prevention(disk, subject, id, reason)
    } else {
        volume.prepare_cancellation(disk, subject, id)
    }
    .map_err(reply::error)
}

impl Server {
    /// Only the active controller or an authorized queue ticket may call this.
    /// It cannot publish file data or obtain authority from a retained identity.
    #[inline(never)]
    pub(super) fn prevent_admission_active(
        &mut self,
        disk: &mut impl PollDisk,
        subject: u64,
        id: AdmissionId,
        reason: PreventionReason,
        active: &mut ActiveExecution,
        control: &mut impl FnMut(&mut Clients, &mut ActiveExecution) -> u64,
    ) -> Result<AdmissionStatus, Error> {
        active.stop();
        let write = publication(&mut self.volume, disk, subject, id, reason)?;
        let retired = drive_stoppable(
            &mut self.clients,
            write,
            None,
            &mut |clients, phase, pending| {
                active.observe(phase, pending, true);
                (control(clients, active), false)
            },
        )?
        .result
        .ok_or(Error::Uncertain)?;
        if retired.state != AdmissionState::Cancelled {
            return Err(Error::Uncertain);
        }
        Ok(retired)
    }
}
