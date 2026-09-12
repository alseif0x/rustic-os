// SPDX-License-Identifier: Apache-2.0
use super::Caller;
use crate::{Clients, Grant, Server, reply};
use rustic_abi::files::{Error, INSPECT_RIGHT};
use rustic_fs::{Admission, AdmissionId, AdmissionStatus, Replacement};

impl Caller {
    pub(super) fn check(self, clients: &Clients, now: u64) -> Result<Grant, Error> {
        self.check_right(clients, now, INSPECT_RIGHT)
    }
    pub(super) fn check_right(
        self,
        clients: &Clients,
        now: u64,
        right: u8,
    ) -> Result<Grant, Error> {
        let grant = clients.grant_at(self.slot).ok_or(Error::Revoked)?;
        grant.check(self.peer, self.context, now)?;
        if grant.subject == 0 || grant.rights & right != right {
            return Err(Error::Denied);
        }
        Ok(grant)
    }
}

impl Server {
    pub(super) fn inspect_admission(
        &self,
        grant: Grant,
        id: AdmissionId,
    ) -> Result<Admission<'_>, Error> {
        let old = self
            .volume
            .admission_by_id(grant.subject, id)
            .map_err(reply::error)?;
        grant
            .operation_inspect(&self.volume, old.request.workspace, old.request.id)
            .map_err(|_| Error::OutcomeUnknown)?;
        Ok(old)
    }

    /// Read-only status under fresh authority; never resumes retained work.
    pub fn admission_status(
        &self,
        caller: Caller,
        id: AdmissionId,
        now: u64,
    ) -> Result<AdmissionStatus, Error> {
        let grant = caller.check(&self.clients, now)?;
        Ok(self.inspect_admission(grant, id)?.status)
    }

    pub(super) fn admission_authorize(
        &self,
        grant: Grant,
        request: Replacement,
    ) -> Result<bool, Error> {
        match self
            .volume
            .admission_by_retry(grant.subject, request.workspace, request.retry)
        {
            Ok(old) => {
                grant
                    .operation_inspect(&self.volume, old.request.workspace, old.request.id)
                    .map_err(|_| Error::OutcomeUnknown)?;
                Ok(false)
            }
            Err(rustic_fs::Error::OutcomeUnknown) => {
                grant.access(&self.volume, request.id, true)?;
                self.volume
                    .resolve(request.workspace, request.id)
                    .map_err(|_| Error::Denied)?;
                Ok(true)
            }
            Err(error) => Err(reply::error(error)),
        }
    }
}
