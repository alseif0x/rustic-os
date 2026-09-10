// SPDX-License-Identifier: Apache-2.0
use super::state::State;
use crate::abi::ipc::Error as IpcError;
use crate::{
    Error,
    ipc::{Endpoint, Message},
    runtime,
};
pub struct Rpc<P = super::Blocking> {
    pub progress: P,
    pub endpoint: Endpoint,
    peer: u64,
    state: State,
}
impl Rpc {
    pub fn new(token: u64, peer: u64) -> Self {
        Self::with_progress(token, peer, super::Blocking)
    }
}
impl<P: super::Progress> Rpc<P> {
    pub fn with_progress(token: u64, peer: u64, progress: P) -> Self {
        Self {
            endpoint: Endpoint::from_bootstrap(token),
            peer,
            state: State::default(),
            progress,
        }
    }
    pub fn rebind(&mut self, token: u64, peer: u64) {
        let old = core::mem::replace(&mut self.endpoint, Endpoint::from_bootstrap(token));
        let _ = old.close();
        self.peer = peer;
        self.state = State::default();
    }
    pub fn pending(&self) -> bool {
        self.state.pending()
    }
    pub fn failed(&self) -> bool {
        self.state.failed()
    }
    /// A single nonblocking send. WouldBlock means this call was not admitted.
    pub fn begin(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if self.failed() {
            return Err(Error::Protocol);
        }
        let correlation = self
            .state
            .ticket()
            .ok_or(Error::Ipc(IpcError::WouldBlock))?;
        let message = Message::new(correlation, bytes)?;
        match self.endpoint.send(&message) {
            Ok(()) => {
                self.state.sent(correlation);
                Ok(())
            }
            Err(Error::Ipc(IpcError::WouldBlock)) => Err(Error::Ipc(IpcError::WouldBlock)),
            Err(e) => {
                self.state.fail();
                Err(e)
            }
        }
    }
    /// Poll at most one reply. No timer, loop, resubmission or retained user pointer.
    pub fn poll(&mut self) -> Result<Option<Message>, Error> {
        if self.failed() {
            return Err(Error::Protocol);
        }
        if !self.pending() {
            return Ok(None);
        }
        match self.endpoint.receive() {
            Ok(reply) => {
                if self.peer != 0 && reply.sender() != self.peer {
                    self.state.fail();
                    return Err(Error::Protocol);
                }
                if !self.state.accept(reply.correlation()) {
                    return Err(Error::Protocol);
                }
                self.peer = reply.sender();
                Ok(Some(reply))
            }
            Err(Error::Ipc(IpcError::WouldBlock)) => Ok(None),
            Err(e) => {
                self.state.fail();
                Err(e)
            }
        }
    }
    /// Synchronous convenience for bounded foreground work. A timeout poisons the
    /// binding so its late reply cannot be mistaken for a subsequent mutation.
    pub fn exchange(&mut self, bytes: &[u8]) -> Result<Message, Error> {
        if self.pending() {
            return Err(Error::Ipc(IpcError::WouldBlock));
        }
        let deadline = runtime::clock().saturating_add(1000);
        loop {
            match self.begin(bytes) {
                Ok(()) => break,
                Err(Error::Ipc(IpcError::WouldBlock)) if runtime::clock() < deadline => {
                    self.progress.wait(self.endpoint.token())?;
                }
                Err(e) => return Err(e),
            }
        }
        loop {
            if let Some(reply) = self.poll()? {
                return Ok(reply);
            }
            if runtime::clock() >= deadline {
                self.state.fail();
                return Err(Error::Protocol);
            }
            if let Err(error) = self.progress.wait(self.endpoint.token()) {
                self.state.fail();
                return Err(error);
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
