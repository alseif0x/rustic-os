// SPDX-License-Identifier: Apache-2.0
use super::{Caller, control::drive};
use crate::{Clients, Server, reply};
use rustic_abi::files::{CANCEL_RIGHT, Error};
use rustic_fs::{AdmissionId, AdmissionStatus, PollDisk};

impl Server {
    /// Cancel a retained admission using independent current cancellation authority.
    /// A committed result is immutable. This endpoint does not preempt another
    /// public request already executing in the native service's dispatch loop.
    #[inline(never)]
    pub fn cancel_admission_with(
        &mut self,
        disk: &mut impl PollDisk,
        caller: Caller,
        id: AdmissionId,
        mut control: impl FnMut(&mut Clients, bool) -> u64,
    ) -> Result<AdmissionStatus, Error> {
        let now = control(&mut self.clients, false);
        let grant = caller.check_right(&self.clients, now, CANCEL_RIGHT)?;
        let old = self
            .volume
            .admission_by_id(grant.subject, id)
            .map_err(reply::error)?;
        grant
            .operation_scope(
                &self.volume,
                old.request.workspace,
                old.request.id,
                CANCEL_RIGHT,
            )
            .map_err(|_| Error::OutcomeUnknown)?;
        let write = self
            .volume
            .prepare_cancellation(disk, grant.subject, id)
            .map_err(reply::error)?;
        let settled = drive(
            &mut self.clients,
            write,
            Some((caller, CANCEL_RIGHT)),
            &mut control,
        )?;
        if let Some(error) = settled.denied {
            // A published cancellation cannot be undone by losing its reply authority.
            return Err(if settled.result.is_some() {
                Error::Uncertain
            } else {
                error
            });
        }
        settled.result.ok_or(Error::Uncertain)
    }
}
