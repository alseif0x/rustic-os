// SPDX-License-Identifier: Apache-2.0
use crate::{Grant, Server, reply};
use rustic_abi::files::{Error, INSPECT_RIGHT, operation::Replacement};
impl Grant {
    pub(crate) fn operation_inspect(
        &self,
        volume: &rustic_fs::Volume,
        workspace: u32,
        object: u32,
    ) -> Result<(), Error> {
        self.operation_scope(volume, workspace, object, INSPECT_RIGHT)
    }
    pub(crate) fn operation_scope(
        &self,
        volume: &rustic_fs::Volume,
        workspace: u32,
        object: u32,
        right: u8,
    ) -> Result<(), Error> {
        // The retained workspace is a storage fact. Never use a caller-claimed
        // workspace as proof that a historical object belonged to this grant.
        if self.subject == 0
            || self.rights & right == 0
            || !(self.scope == 0
                || self.scope == object
                || self.scope == workspace
                || volume.within(workspace, self.scope)
                || volume.within(object, self.scope))
        {
            return Err(Error::Denied);
        }
        Ok(())
    }
}
impl Server {
    pub(super) fn operation_authorize(
        &self,
        grant: Grant,
        request: Replacement,
    ) -> Result<bool, Error> {
        if grant.subject == 0 || grant.rights & INSPECT_RIGHT == 0 {
            return Err(Error::Denied);
        }
        match self.volume.operation_by_retry(
            grant.subject,
            request.workspace.root(),
            super::query::stored(request.workspace, request.retry),
        ) {
            Ok(old) => {
                grant.operation_inspect(&self.volume, old.workspace, old.receipt.id)?;
                Ok(false)
            }
            Err(rustic_fs::Error::OutcomeUnknown) => {
                grant.access(&self.volume, request.resource.object(), true)?;
                self.volume
                    .resolve(request.workspace.root(), request.resource.object())
                    .map_err(|_| Error::Denied)?;
                Ok(true)
            }
            Err(error) => Err(reply::error(error)),
        }
    }
}
