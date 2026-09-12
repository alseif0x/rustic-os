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
    /// Explicit profile 2; legacy causes stay Unknown. Never falls back, resumes
    /// execution or retries a mutation when the profile is unsupported.
    pub fn admission_observe_v2(&mut self, id: AdmissionId) -> Result<a::ObservationV2, Error> {
        let reply = self.operation_exchange(a::ObservationV2::request(id, self.context)?)?;
        let result = a::ObservationV2::decode(&reply)?;
        if result.coarse().id() != id {
            return Err(Error::Protocol);
        }
        Ok(result)
    }
    /// Read either live progress or a retained fact in one exchange. The same ID
    /// remains valid after settlement/restart while retained. Never resumes work.
    pub fn admission_observe(&mut self, id: AdmissionId) -> Result<a::Observation, Error> {
        let reply = self.operation_exchange(id.packet(a::OBSERVE, self.context)?)?;
        let result = a::Observation::decode(&reply)?;
        if result.id() != id {
            return Err(Error::Protocol);
        }
        Ok(result)
    }
    /// Schedule an already durable admission and return before settlement.
    /// A lost reply is uncertain; inspect activity/durable status before deciding
    /// whether to explicitly resubmit. Restart never resumes a volatile schedule.
    pub fn admission_schedule(&mut self, id: AdmissionId) -> Result<a::Activity, Error> {
        self.admission_live(id, a::SCHEDULE)
    }
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
        let error = if matches!(op, a::REQUEST_CANCEL | a::SCHEDULE) {
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
