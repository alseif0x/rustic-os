// SPDX-License-Identifier: Apache-2.0
use super::{
    Error, Message,
    channel::Channel,
    handles::{Endpoint, Table},
};
use rustic_abi::ipc::{ALL, READ, WRITE};

pub struct Broker {
    channels: [Option<Channel>; 4],
    handles: Table,
    next_channel: u64,
}
impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}
impl Broker {
    pub const fn new() -> Self {
        Self {
            channels: [const { None }; 4],
            handles: Table::new(),
            next_channel: 1,
        }
    }
    fn slot(&self, endpoint: Endpoint) -> usize {
        self.channels
            .iter()
            .position(|c| c.as_ref().is_some_and(|c| c.id == endpoint.channel))
            .expect("live handle owns channel")
    }
    pub fn connect(&mut self, a: u64, b: u64) -> Result<(u64, u64), Error> {
        let slot = self
            .channels
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Quota)?;
        let next = self.next_channel.checked_add(1).ok_or(Error::Quota)?;
        let endpoint = Endpoint {
            channel: self.next_channel,
            side: 0,
        };
        let first = self.handles.grant(a, endpoint, ALL)?;
        let second = match self.handles.grant(
            b,
            Endpoint {
                side: 1,
                ..endpoint
            },
            ALL,
        ) {
            Ok(id) => id,
            Err(error) => {
                self.handles.remove(a, first)?;
                return Err(error);
            }
        };
        self.channels[slot] = Some(Channel::new(self.next_channel));
        self.next_channel = next;
        Ok((first, second))
    }
    pub fn check(&self, owner: u64, handle: u64, right: u8) -> Result<(), Error> {
        self.handles.resolve(owner, handle, right).map(|_| ())
    }
    pub fn send(&mut self, owner: u64, handle: u64, bytes: &[u8]) -> Result<(), Error> {
        let endpoint = self.handles.resolve(owner, handle, WRITE)?;
        let message = Message::decode(bytes, owner)?;
        let slot = self.slot(endpoint);
        self.channels[slot]
            .as_mut()
            .unwrap()
            .send(endpoint.side, message)
    }
    pub fn peek(&self, owner: u64, handle: u64) -> Result<Message, Error> {
        let endpoint = self.handles.resolve(owner, handle, READ)?;
        self.channels[self.slot(endpoint)]
            .as_ref()
            .unwrap()
            .peek(endpoint.side)
            .copied()
    }
    /// Caller commits only after successful copyout; no user runs between peek/pop.
    pub fn consume(&mut self, owner: u64, handle: u64) -> Result<(), Error> {
        let endpoint = self.handles.resolve(owner, handle, READ)?;
        let slot = self.slot(endpoint);
        self.channels[slot].as_ref().unwrap().peek(endpoint.side)?;
        self.channels[slot].as_mut().unwrap().pop(endpoint.side);
        Ok(())
    }
    pub fn close(&mut self, owner: u64, handle: u64) -> Result<(), Error> {
        let endpoint = self.handles.remove(owner, handle)?;
        let slot = self.slot(endpoint);
        if self.channels[slot].as_mut().unwrap().close(endpoint.side) {
            self.channels[slot] = None;
        }
        Ok(())
    }
    pub fn close_owner(&mut self, owner: u64) {
        while let Some(handle) = self.handles.first(owner) {
            self.close(owner, handle).expect("owned live handle");
        }
    }
    /// Explicit move for the trusted launcher; rights may only decrease.
    pub fn transfer(
        &mut self,
        owner: u64,
        handle: u64,
        target: u64,
        rights: u8,
    ) -> Result<u64, Error> {
        self.handles.transfer(owner, handle, target, rights)
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.channels.iter().flatten().count(), self.handles.count())
    }
}
