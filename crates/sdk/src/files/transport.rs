// SPDX-License-Identifier: Apache-2.0
//! Correlated native packet transport and uncertainty classification.
use super::Client;
use rustic_abi::files::*;
impl<P: crate::rpc::Progress> Client<P> {
    /// One asynchronous request; the client owns the original opcode/context until collection.
    pub fn submit(&mut self, mut p: Packet) -> Result<(), Error> {
        self.require_binding()?;
        p.context = self.context;
        self.rpc.begin(&p.encode()).map_err(|e| match e {
            crate::Error::Ipc(crate::abi::ipc::Error::WouldBlock) => Error::Busy,
            _ => Error::Closed,
        })?;
        self.pending = Some(p);
        Ok(())
    }
    pub fn poll(&mut self) -> Result<Option<Packet>, Error> {
        let Some(p) = self.pending else {
            return Ok(None);
        };
        let durable = matches!(p.op, CREATE | MKDIR | REMOVE | COMMIT);
        let error = if durable {
            Error::Uncertain
        } else {
            Error::Protocol
        };
        let response = self.rpc.poll().map_err(|_| error)?;
        let Some(message) = response else {
            return Ok(None);
        };
        self.pending = None;
        let reply = Packet::decode(message.payload()).map_err(|_| error)?;
        if reply.op != p.op || reply.context != p.context {
            return Err(error);
        }
        Error::parse(reply.status)?;
        Ok(Some(reply))
    }
    pub fn request(&mut self, mut p: Packet) -> Result<Packet, Error> {
        self.require_binding()?;
        p.context = self.context;
        let durable = matches!(p.op, CREATE | MKDIR | REMOVE | COMMIT);
        let malformed = if durable {
            Error::Uncertain
        } else {
            Error::Protocol
        };
        let message = self.rpc.exchange(&p.encode()).map_err(|e| match e {
            _ if durable => Error::Uncertain,
            crate::Error::Interrupted => Error::Interrupted,
            crate::Error::Ipc(crate::abi::ipc::Error::Closed) => Error::Closed,
            _ => Error::Protocol,
        })?;
        let reply = Packet::decode(message.payload()).map_err(|_| malformed)?;
        if reply.op != p.op || reply.context != p.context {
            return Err(malformed);
        }
        Error::parse(reply.status)?;
        Ok(reply)
    }
}
