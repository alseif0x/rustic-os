// SPDX-License-Identifier: Apache-2.0
//! Explicit durable preparation, inspection, execution and cancellation.
use super::Client;
use rustic_abi::files::{
    admission::{self as a, AdmissionId, State, Status},
    operation::{Lookup, Replacement, Retry},
    reference::Workspace,
    *,
};

impl<P: crate::rpc::Progress> Client<P> {
    /// Live observation only. Unavailable means this service is not currently
    /// executing a matching operation; recover durable facts with admission_get.
    pub fn admission_activity(&mut self, id: AdmissionId) -> Result<a::Activity, Error> {
        self.admission_live(id, a::ACTIVITY)
    }
    /// Volatile accepted stop, not a durable Cancelled result. Never auto-retry.
    pub fn admission_request_cancel(&mut self, id: AdmissionId) -> Result<a::Activity, Error> {
        self.admission_live(id, a::REQUEST_CANCEL)
    }
    fn admission_live(&mut self, id: AdmissionId, op: u8) -> Result<a::Activity, Error> {
        let error = if op == a::REQUEST_CANCEL {
            Error::Uncertain
        } else {
            Error::Protocol
        };
        let result = a::Activity::decode(&self.operation_exchange(id.packet(op, self.context)?)?)
            .map_err(|_| error)?;
        if result.id != id {
            return Err(error);
        }
        Ok(result)
    }
    /// Volatile staging; only a later ACCEPT can acknowledge durability.
    pub fn stage_admission(&mut self, request: Replacement, bytes: &[u8]) -> Result<(), Error> {
        self.stage_profile(request, bytes, true)
    }
    /// Save the workspace/retry tuple first. Missing acceptance is uncertain;
    /// recover with admission_retry. Never automatically executes or replays.
    pub fn admit_file(&mut self, request: Replacement, bytes: &[u8]) -> Result<Status, Error> {
        self.stage_profile(request, bytes, true)?;
        let mut p = Packet::new(a::ACCEPT);
        p.id = request.resource.object();
        let result = Status::decode(&self.operation_exchange(p)?).map_err(|_| Error::Uncertain)?;
        if result.id.lineage() != request.workspace.lineage() {
            return Err(Error::Uncertain);
        }
        Ok(result)
    }
    pub fn admission_retry(&mut self, workspace: Workspace, retry: Retry) -> Result<Status, Error> {
        let mut p = Lookup::Retry { workspace, retry }.packet(self.context);
        p.op = a::RETRY;
        let result = Status::decode(&self.operation_exchange(p)?)?;
        if result.id.lineage() != workspace.lineage() {
            return Err(Error::Protocol);
        }
        Ok(result)
    }
    pub fn admission_get(&mut self, id: AdmissionId) -> Result<Status, Error> {
        self.admission_action(id, a::GET)
    }
    /// Rechecks live write authority and the expected file version at execution.
    pub fn admission_execute(&mut self, id: AdmissionId) -> Result<Status, Error> {
        self.admission_action(id, a::EXECUTE)
    }
    /// CANCEL authority is independent of READ/WRITE/INSPECT. A Committed result
    /// means cancellation was too late. Repeating Cancelled is a read-only no-op.
    pub fn admission_cancel(&mut self, id: AdmissionId) -> Result<Status, Error> {
        self.admission_action(id, a::CANCEL)
    }
    fn admission_action(&mut self, id: AdmissionId, op: u8) -> Result<Status, Error> {
        let error = if a::controlled(op) {
            Error::Uncertain
        } else {
            Error::Protocol
        };
        let result = Status::decode(&self.operation_exchange(id.packet(op, self.context)?)?)
            .map_err(|_| error)?;
        if result.id != id || a::controlled(op) && result.state == State::Admitted {
            return Err(error);
        }
        Ok(result)
    }
}
