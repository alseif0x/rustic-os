// SPDX-License-Identifier: Apache-2.0
use super::{Caller, control::drive};
use crate::{Clients, Server, reply};
use rustic_abi::files::Error;
use rustic_fs::{AdmissionId, AdmissionState as State, AdmissionStatus, PollDisk, Replacement};

impl Server {
    /// Persist a replacement's full arguments without executing it. An identical
    /// retained retry is inspection only. The callback has the same bounded,
    /// trusted-owner contract as commit_with; this is not a public IPC endpoint.
    #[inline(never)]
    pub fn admit_with(
        &mut self,
        disk: &mut impl PollDisk,
        caller: Caller,
        request: Replacement,
        bytes: &[u8],
        mut control: impl FnMut(&mut Clients, bool) -> u64,
    ) -> Result<AdmissionStatus, Error> {
        let now = control(&mut self.clients, false);
        let grant = caller.check(&self.clients, now)?;
        let new = self.admission_authorize(grant, request)?;
        let write = self
            .volume
            .prepare_admission(disk, grant.subject, self.instance, request, bytes)
            .map_err(reply::error)?;
        let settled = drive(
            &mut self.clients,
            write,
            Some((caller, rustic_abi::files::INSPECT_RIGHT)),
            &mut control,
        )?;
        if new && let Some(status) = settled.result {
            if self.instance == 0 {
                self.instance = status.id.number;
            }
            if settled.denied.is_some() {
                // Admission crossed its header after revocation. No file effect
                // started; record terminal prevention before returning a denial.
                self.retire_admission(disk, grant.subject, status.id, &mut control)?;
            }
        }
        if let Some(error) = settled.denied {
            return Err(error);
        }
        settled.result.ok_or(Error::Uncertain)
    }

    /// Execute an explicit retained admission under fresh live write authority.
    /// Lookup/remount/retry never call this. Early revocation drains data I/O and
    /// persists terminal cancellation; late revocation settles but withholds success.
    #[inline(never)]
    pub fn execute_admission_with(
        &mut self,
        disk: &mut impl PollDisk,
        caller: Caller,
        id: AdmissionId,
        mut control: impl FnMut(&mut Clients, bool) -> u64,
    ) -> Result<AdmissionStatus, Error> {
        let now = control(&mut self.clients, false);
        let grant = caller.check(&self.clients, now)?;
        let old = self.inspect_admission(grant, id)?;
        if old.status.state != State::Admitted {
            return Ok(old.status);
        }
        grant.access(&self.volume, old.request.id, true)?;
        let write = self
            .volume
            .prepare_admitted(disk, grant.subject, id)
            .map_err(reply::error)?;
        let settled = drive(
            &mut self.clients,
            write,
            Some((caller, rustic_abi::files::INSPECT_RIGHT)),
            &mut control,
        )?;
        if let Some(error) = settled.denied {
            if settled.result.is_some() {
                return Err(Error::Uncertain);
            }
            self.retire_admission(disk, grant.subject, id, &mut control)?;
            return Err(error);
        }
        Ok(self
            .volume
            .admission_by_id(grant.subject, id)
            .map_err(reply::error)?
            .status)
    }

    /// Service housekeeping, never delegated cancellation authority. The only
    /// callers know that a newly admitted/file operation was prevented by owner
    /// revocation, expiry or detach. Ordinary retries cannot reach this path.
    #[inline(never)]
    fn retire_admission(
        &mut self,
        disk: &mut impl PollDisk,
        subject: u64,
        id: AdmissionId,
        control: &mut impl FnMut(&mut Clients, bool) -> u64,
    ) -> Result<(), Error> {
        let write = self
            .volume
            .prepare_cancellation(disk, subject, id)
            .map_err(reply::error)?;
        let status = drive(&mut self.clients, write, None, control)?
            .result
            .ok_or(Error::Uncertain)?;
        if status.state != State::Cancelled {
            return Err(Error::Uncertain);
        }
        Ok(())
    }
}
