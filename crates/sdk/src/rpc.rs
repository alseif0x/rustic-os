// SPDX-License-Identifier: Apache-2.0
//! Bounded synchronous requests. Every response binds correlation and authentic peer.
use crate::{
    Error,
    ipc::{Endpoint, Message},
    runtime,
};
pub struct Rpc {
    pub endpoint: Endpoint,
    peer: u64,
    next: u64,
}
impl Rpc {
    pub fn new(token: u64, peer: u64) -> Self {
        Self {
            endpoint: Endpoint::from_bootstrap(token),
            peer,
            next: 1,
        }
    }
    pub fn exchange(&mut self, bytes: &[u8]) -> Result<Message, Error> {
        let correlation = self.next;
        self.next = self.next.checked_add(1).ok_or(Error::Protocol)?;
        let message = Message::new(correlation, bytes)?;
        let deadline = runtime::clock().saturating_add(1000);
        loop {
            match self.endpoint.send(&message) {
                Ok(()) => break,
                Err(Error::Ipc(crate::abi::ipc::Error::WouldBlock))
                    if runtime::clock() < deadline => {}
                Err(e) => return Err(e),
            }
        }
        loop {
            match self.endpoint.receive() {
                Ok(reply) => {
                    if reply.correlation() != correlation
                        || (self.peer != 0 && reply.sender() != self.peer)
                    {
                        return Err(Error::Protocol);
                    }
                    // Bootstrap peer 0 discovers the actual sender on an already owner-bound channel.
                    self.peer = reply.sender();
                    return Ok(reply);
                }
                Err(Error::Ipc(crate::abi::ipc::Error::WouldBlock)) => {
                    if runtime::clock() >= deadline {
                        return Err(Error::Protocol);
                    }
                    runtime::wait_set(&[self.endpoint.token()], 100)
                        .map_err(|_| Error::Protocol)?;
                }
                Err(e) => return Err(e),
            }
        }
    }
    pub fn words(&mut self, words: [u64; 8]) -> Result<[u64; 8], Error> {
        crate::abi::runtime::decode(
            self.exchange(&crate::abi::runtime::encode(words))?
                .payload(),
        )
        .map_err(|_| Error::Protocol)
    }
}
