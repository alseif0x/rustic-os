// SPDX-License-Identifier: Apache-2.0
//! Profile-2 staged admission for large workspace files: the same streamed
//! chunking as a tracked replacement, finished by an explicit ACCEPT that
//! makes the admission durable without executing it, and status lookup by
//! retry identity with the profile-2 marker.
//!
//! Status by admission ID, execution, cancellation and observation use the
//! profile-independent requests in the admissions client
//! ([`Client::admission_get`], [`Client::admission_execute`],
//! [`Client::admission_cancel`], [`Client::admission_observe_v2`]).
use super::Client;
use super::workspace::{WorkspaceTransfer, empty_ack};
use rustic_abi::files::{
    admission::{self as a, Status},
    operation::{self, Retry},
    reference::Workspace,
    workspace::{Lookup, MAX_FILE_BYTES, Replacement},
    *,
};

impl<P: crate::rpc::Progress> Client<P> {
    /// Stage and accept one admission of `size` bytes produced by
    /// `fill(offset, buffer)`. Nothing executes until an explicit
    /// [`Client::admission_execute`].
    ///
    /// Retain the workspace, retry epoch and key before calling: an
    /// [`Error::Uncertain`] from ACCEPT (see [`Self::workspace_accept`])
    /// leaves the outcome unknown; recover with
    /// [`Self::admission_retry7`], or repeat the call with the same identity
    /// and bytes, which reports the retained admission without a second one.
    /// A failure before ACCEPT aborts the service transfer.
    pub fn workspace_admit(
        &mut self,
        request: operation::Replacement,
        size: u32,
        mut fill: impl FnMut(u32, &mut [u8]) -> Result<(), Error>,
    ) -> Result<Status, Error> {
        let mut transfer = self.workspace_admission_open(request, size)?;
        while transfer.offset() < transfer.size() {
            if let Err(error) = self.workspace_chunk(&mut transfer, &mut fill) {
                let _ = self.workspace_abort(transfer);
                return Err(error);
            }
        }
        self.workspace_accept(transfer)
    }

    /// Open a profile-2 admission transfer of `size` bytes. Send its bytes
    /// with [`Self::workspace_chunk`], then [`Self::workspace_accept`] or
    /// [`Self::workspace_abort`].
    pub fn workspace_admission_open(
        &mut self,
        request: operation::Replacement,
        size: u32,
    ) -> Result<WorkspaceTransfer, Error> {
        if size > MAX_FILE_BYTES {
            return Err(Error::Size);
        }
        let mut open = Replacement { request }.packet(size as usize, self.context)?;
        open.op = a::OPEN;
        empty_ack(self.operation_exchange(open)?)?;
        Ok(WorkspaceTransfer::new(request, size, true))
    }

    /// Make a complete admission transfer durable and return its status. An
    /// incomplete transfer is aborted and refused with [`Error::Offset`]; a
    /// tracked transfer is aborted and refused with [`Error::Protocol`].
    ///
    /// Once ACCEPT is sent, only an outcome the client cannot determine is
    /// [`Error::Uncertain`]: a transport failure after sending, a malformed
    /// status reply or a status of another lineage. A refusal the service
    /// reports (for example `Version`, `Full` or `IdempotencyConflict`, all
    /// decided before any publication I/O) propagates as that error; a disk
    /// failure during the publication is reported by the service as
    /// `Uncertain`.
    pub fn workspace_accept(&mut self, transfer: WorkspaceTransfer) -> Result<Status, Error> {
        if !transfer.admission() {
            let _ = self.workspace_abort(transfer);
            return Err(Error::Protocol);
        }
        if transfer.offset() != transfer.size() {
            let _ = self.workspace_abort(transfer);
            return Err(Error::Offset);
        }
        let request = transfer.request();
        let mut p = Packet::new(a::ACCEPT);
        p.id = request.resource.object();
        let result = Status::decode(&self.operation_exchange(p)?).map_err(|_| Error::Uncertain)?;
        if result.id.lineage() != request.workspace.lineage() {
            return Err(Error::Uncertain);
        }
        Ok(result)
    }

    /// Status of the admission with this profile-2 retry identity, within the
    /// grant's own subject and scope. Never executes or resumes it.
    pub fn admission_retry7(
        &mut self,
        workspace: Workspace,
        retry: Retry,
    ) -> Result<Status, Error> {
        let mut p = Lookup {
            query: operation::Lookup::Retry { workspace, retry },
        }
        .packet(self.context);
        p.op = a::RETRY;
        let result = Status::decode(&self.operation_exchange(p)?)?;
        if result.id.lineage() != workspace.lineage() {
            return Err(Error::Protocol);
        }
        Ok(result)
    }
}
