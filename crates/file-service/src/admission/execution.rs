// SPDX-License-Identifier: Apache-2.0
//! Explicit execution with bounded public control between owned publication polls.
mod publication;
use super::{ActiveExecution, Caller};
use crate::{Clients, Server, reply};
use rustic_abi::files::Error;
use rustic_fs::{AdmissionId, AdmissionState, AdmissionStatus, PollDisk};

impl Server {
    /// Callback may poll bounded transport/control only, never issue grants or
    /// mutate storage. REQUEST_CANCEL latches a volatile stop under current
    /// cancellation authority; durable prevention requires later settlement.
    #[inline(never)]
    pub fn execute_admission_active_with(
        &mut self,
        disk: &mut impl PollDisk,
        caller: Caller,
        id: AdmissionId,
        now: u64,
        mut control: impl FnMut(&mut Clients, &mut ActiveExecution) -> u64,
    ) -> Result<AdmissionStatus, Error> {
        let grant = caller.check(&self.clients, now)?;
        let old = self.inspect_admission(grant, id)?;
        if old.status.state != AdmissionState::Admitted {
            return Ok(old.status);
        }
        grant.access(&self.volume, old.request.id, true)?;
        let mut active = ActiveExecution::new(self, grant.subject, old)?;
        let settled = self.publish_admission_active(
            disk,
            caller,
            grant.subject,
            id,
            &mut active,
            &mut control,
        )?;
        if settled.result.is_none() {
            self.prevent_admission_active(disk, grant.subject, id, &mut active, &mut control)?;
        }
        if let Some(error) = settled.denied {
            return Err(if settled.result.is_some() {
                Error::Uncertain
            } else {
                error
            });
        }
        // An owner revoke/expiry during terminal housekeeping must not let an old
        // execution caller receive its result under dead authority.
        let now = control(&mut self.clients, &mut active);
        caller
            .check(&self.clients, now)
            .map_err(|_| Error::Uncertain)?;
        Ok(self
            .volume
            .admission_by_id(grant.subject, id)
            .map_err(reply::error)?
            .status)
    }
}
