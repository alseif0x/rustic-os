// SPDX-License-Identifier: Apache-2.0
//! Explicit execution with bounded public control between owned publication polls.
use super::{ActiveExecution, Caller, control::drive_stoppable};
use crate::{Clients, Server, reply};
use rustic_abi::files::{Error, INSPECT_RIGHT};
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
        let write = self
            .volume
            .prepare_admitted(disk, grant.subject, id)
            .map_err(reply::error)?;
        let settled = drive_stoppable(
            &mut self.clients,
            write,
            Some((caller, INSPECT_RIGHT)),
            &mut |clients, phase, pending| {
                active.observe(phase, pending, false);
                let now = control(clients, &mut active);
                (now, active.stopping())
            },
        )?;
        if settled.result.is_none() {
            // Publication was prevented. Finish the service-owned terminal record
            // even after requester loss. A prior accepted stop is not undone by
            // later revocation. No speculative Cancelled result is disclosed.
            active.stop();
            let write = self
                .volume
                .prepare_cancellation(disk, grant.subject, id)
                .map_err(reply::error)?;
            let retired = drive_stoppable(
                &mut self.clients,
                write,
                None,
                &mut |clients, phase, pending| {
                    active.observe(phase, pending, true);
                    (control(clients, &mut active), false)
                },
            )?
            .result
            .ok_or(Error::Uncertain)?;
            if retired.state != AdmissionState::Cancelled {
                return Err(Error::Uncertain);
            }
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
