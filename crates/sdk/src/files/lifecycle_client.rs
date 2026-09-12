// SPDX-License-Identifier: Apache-2.0
//! Single-exchange service-v2 bindings; no read-then-stop authority escalation.
use super::{
    Client, Error,
    admission::AdmissionId,
    lifecycle::{CancelAck, Operation},
};

impl<P: crate::rpc::Progress> Client<P> {
    pub fn operation_inspect(&mut self, id: AdmissionId) -> Result<Operation, Error> {
        self.admission_observe_v2(id)?.try_into()
    }
    /// Volatile service acceptance only. A lost/malformed reply is uncertain;
    /// callers must explicitly reconcile, never infer prevention or auto-retry.
    pub fn operation_cancel(&mut self, id: AdmissionId) -> Result<CancelAck, Error> {
        let reply = self.operation_exchange(CancelAck::request(id, self.context)?)?;
        let ack = CancelAck::decode(&reply).map_err(|_| Error::Uncertain)?;
        if ack.id != id {
            return Err(Error::Uncertain);
        }
        Ok(ack)
    }
}
