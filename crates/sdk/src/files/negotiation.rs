// SPDX-License-Identifier: Apache-2.0
//! A selected method borrows its live client: it cannot survive a rebind or move
//! to another client. Support is a snapshot; every effect still checks authority.
use super::Client;
use rustic_abi::{
    files::{
        Error,
        admission::AdmissionId,
        lifecycle::{CancelAck, Operation},
        negotiation::{self, Descriptor},
    },
    services::{Availability, Method},
};

pub struct LifecycleBinding<'a, P: crate::rpc::Progress = crate::rpc::Blocking> {
    client: &'a mut Client<P>,
    descriptor: Descriptor,
    responder: u64,
}

impl<P: crate::rpc::Progress> Client<P> {
    /// Select one reviewed service-v2 method on this exact native connection.
    /// Unknown versions/digests are refused, never downgraded or retried.
    ///
    /// ```compile_fail
    /// use rustic_sdk::{files::Client, abi::services::Method};
    /// let mut client = Client::new(1, 2, 3);
    /// let mut selected = client.negotiate_lifecycle(Method::OperationsGet).unwrap();
    /// client.rebind(4, 5, 6); // the selection exclusively borrows this binding
    /// let _ = selected.descriptor();
    /// ```
    pub fn negotiate_lifecycle(
        &mut self,
        method: Method,
    ) -> Result<LifecycleBinding<'_, P>, Error> {
        let reply = self.operation_exchange(negotiation::request(method, self.context)?)?;
        let descriptor = Descriptor::decode(&reply, method)?;
        let responder = self.rpc.responder();
        if responder == 0 {
            return Err(Error::Protocol);
        }
        Ok(LifecycleBinding {
            client: self,
            descriptor,
            responder,
        })
    }
}

impl<P: crate::rpc::Progress> LifecycleBinding<'_, P> {
    pub fn descriptor(&self) -> Descriptor {
        self.descriptor
    }
    /// Current kernel-authenticated process, meaningful only within this boot
    /// and connection. It is NOT the historical operation service_instance.
    pub fn responder(&self) -> u64 {
        self.responder
    }
    pub fn context(&self) -> u32 {
        self.client.context
    }

    pub fn inspect(&mut self, id: AdmissionId) -> Result<Operation, Error> {
        self.require(Method::OperationsGet)?;
        self.client.operation_inspect(id)
    }
    pub fn cancel(&mut self, id: AdmissionId) -> Result<CancelAck, Error> {
        self.require(Method::OperationsCancel)?;
        self.client.operation_cancel(id)
    }
    fn require(&self, method: Method) -> Result<(), Error> {
        if self.descriptor.method != method {
            return Err(Error::Unsupported);
        }
        if self.descriptor.availability != Availability::Available {
            return Err(Error::Unavailable);
        }
        Ok(())
    }
}
