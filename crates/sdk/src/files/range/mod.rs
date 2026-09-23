// SPDX-License-Identifier: Apache-2.0
//! One logical read, assembled from independently authenticated and correlated exchanges.
mod collector;
mod operation;
use super::{Client, Error, Packet, read};
pub use collector::VerifiedRange;
use collector::read_into;
pub use operation::RangeProgress;
use operation::{Operation, Transport, pending_marker, poll_operation};
use rustic_abi::files::read::Request;

/// An owned, pollable version-pinned read that can be retained across caller ticks.
/// Each poll uses the client only for one send or one receive. Dropping this value
/// clears its owned data buffer; its exclusive client reservation must be drained
/// or rebound before another request can be sent.
/// While it is in flight, other client requests return `Busy`.
/// This is ordinary memory clearing, not a cryptographic erasure guarantee; the
/// SHA-256 implementation's internal scratch state is not explicitly cleared.
pub struct RangeRead {
    operation: Operation,
}

impl<P: crate::rpc::Progress> Client<P> {
    /// Start one bounded range read, pinning the current client token and context.
    pub fn begin_read_range(&mut self, request: Request) -> Result<RangeRead, Error> {
        self.require_binding()?;
        if self.active_range.is_some() || self.pending.is_some() || self.rpc.pending() {
            return Err(Error::Busy);
        }
        let id = self.next_range_id;
        if id == 0 {
            return Err(Error::Unavailable);
        }
        let operation = Operation::new(request, self.token(), self.context, id)?;
        self.next_range_id = id.checked_add(1).unwrap_or(0);
        self.active_range = Some(id);
        Ok(RangeRead { operation })
    }

    /// Abandon the active range, discarding one in-flight reply if necessary.
    /// Returns `false` when that reply is not ready yet. If the transport failed,
    /// rebind the client instead.
    pub fn drain_abandoned_read_range(&mut self) -> Result<bool, Error> {
        let Some(id) = self.active_range else {
            return Err(Error::Unavailable);
        };
        if self.rpc.failed() {
            return Err(Error::Protocol);
        }
        let Some(marker) = self.pending else {
            if self.rpc.pending() {
                return Err(Error::Protocol);
            }
            self.active_range = None;
            return Ok(true);
        };
        if !operation::is_pending_marker(marker)
            || marker.context != self.context
            || marker.version != id
        {
            return Err(Error::Busy);
        }
        if !self.rpc.pending() {
            return Err(Error::Protocol);
        }
        match self.rpc.poll().map_err(map_transport_error)? {
            None => Ok(false),
            Some(_) => {
                self.pending = None;
                self.active_range = None;
                Ok(true)
            }
        }
    }

    /// Synchronous convenience for bounded foreground work. The caller's buffer
    /// is cleared before validation and receives bytes only after hash verification.
    pub fn read_range(&mut self, request: Request, out: &mut [u8]) -> Result<read::Info, Error> {
        let context = self.context;
        read_into(request, context, out, |packet| self.observation(packet))
    }

    // These observations retain the same Rpc peer/correlation stream as legacy
    // file calls, while rejecting noncanonical error responses as well as data.
    pub(super) fn observation(&mut self, mut request: Packet) -> Result<Packet, Error> {
        self.require_binding()?;
        if self.active_range.is_some() || self.pending.is_some() || self.rpc.pending() {
            return Err(Error::Busy);
        }
        request.context = self.context;
        let response = self
            .rpc
            .exchange(&request.encode())
            .map_err(map_transport_error)?;
        Packet::decode(response.payload())?.checked_reply(request.op, self.context)
    }
}

pub(super) fn is_pending_marker(packet: Packet) -> bool {
    operation::is_pending_marker(packet)
}

impl RangeRead {
    /// Attempt one send or one receive poll. `Busy` means a send was not admitted;
    /// another call is an explicit caller decision. An admitted request is never retried.
    pub fn poll<P: crate::rpc::Progress>(
        &mut self,
        client: &mut Client<P>,
    ) -> Result<RangeProgress, Error> {
        poll_operation(&mut self.operation, &mut ClientTransport { client })
    }

    /// Consume the verified result after `poll` reports `Complete`.
    pub fn finish(self) -> Result<VerifiedRange, Error> {
        self.operation.finish()
    }
}

struct ClientTransport<'a, P: crate::rpc::Progress> {
    client: &'a mut Client<P>,
}

impl<P: crate::rpc::Progress> Transport for ClientTransport<'_, P> {
    fn token(&self) -> u64 {
        self.client.token()
    }

    fn context(&self) -> u32 {
        self.client.context
    }

    fn active_range_id(&self) -> Option<u64> {
        self.client.active_range
    }

    fn release_range(&mut self, id: u64) {
        if self.client.active_range == Some(id) {
            self.client.active_range = None;
        }
    }

    fn send(&mut self, packet: Packet) -> Result<(), Error> {
        self.client.require_binding()?;
        let Some(id) = self.client.active_range else {
            return Err(Error::Unavailable);
        };
        if self.client.pending.is_some() || self.client.rpc.pending() {
            return Err(Error::Busy);
        }
        match self.client.rpc.begin(&packet.encode()) {
            Ok(()) => {
                self.client.pending = Some(pending_marker(packet.context, id));
                Ok(())
            }
            Err(crate::Error::Ipc(crate::abi::ipc::Error::WouldBlock)) => Err(Error::Busy),
            Err(error) => Err(map_transport_error(error)),
        }
    }

    fn receive(&mut self, _op: u8, context: u32) -> Result<Option<Packet>, Error> {
        let Some(id) = self.client.active_range else {
            return Err(Error::Protocol);
        };
        if self.client.pending != Some(pending_marker(context, id)) || !self.client.rpc.pending() {
            return Err(Error::Protocol);
        }
        match self.client.rpc.poll().map_err(map_transport_error)? {
            None => Ok(None),
            Some(message) => {
                self.client.pending = None;
                Packet::decode(message.payload()).map(Some)
            }
        }
    }
}

fn map_transport_error(error: crate::Error) -> Error {
    match error {
        crate::Error::Interrupted => Error::Interrupted,
        crate::Error::Ipc(crate::abi::ipc::Error::Closed) => Error::Closed,
        _ => Error::Protocol,
    }
}
