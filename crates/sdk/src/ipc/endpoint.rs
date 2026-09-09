// SPDX-License-Identifier: Apache-2.0
use super::Message;
use crate::{Error, arch, error::decode};
use rustic_abi::ipc as abi;
/// A local handle token. The kernel validates owner, lifetime and rights on every call.
/// Explicit close; process exit also reclaims handles. Never clone or serialize as authority.
pub struct Endpoint(u64);
impl Endpoint {
    /// Wrap a bootstrap token; this does not create a channel or grant access.
    pub fn from_bootstrap(token: u64) -> Self {
        Self(token)
    }
    pub fn send(&self, message: &Message) -> Result<(), Error> {
        if message.sender() != 0 {
            return Err(Error::Ipc(abi::Error::Message));
        }
        // SAFETY: Immutable initialized storage remains alive through synchronous copy-in.
        let result = decode(unsafe {
            arch::call(
                abi::SEND,
                self.0,
                message.wire().as_ptr() as u64,
                message.wire().len() as u64,
            )
        })?;
        if result != 0 {
            return Err(Error::Protocol);
        }
        Ok(())
    }
    pub fn receive(&self) -> Result<Message, Error> {
        let mut bytes = [0; abi::MAX_MESSAGE];
        // SAFETY: Exclusive writable initialized storage, bounded to its allocation;
        // no concurrent access, retained until synchronous kernel copy-out completes.
        let length = decode(unsafe {
            arch::call(
                abi::RECEIVE,
                self.0,
                bytes.as_mut_ptr() as u64,
                bytes.len() as u64,
            )
        })?;
        let length = usize::try_from(length).map_err(|_| Error::Protocol)?;
        Message::from_received(bytes.get(..length).ok_or(Error::Protocol)?)
    }
    pub fn wait(&self) -> Result<(), Error> {
        // SAFETY: WAIT takes an integer token, no pointer or retained Rust borrow.
        if decode(unsafe { arch::call(abi::WAIT, self.0, 0, 0) })? != 0 {
            return Err(Error::Protocol);
        }
        Ok(())
    }
    pub fn close(self) -> Result<(), Error> {
        // SAFETY: CLOSE takes only an integer token.
        if decode(unsafe { arch::call(abi::CLOSE, self.0, 0, 0) })? != 0 {
            return Err(Error::Protocol);
        }
        Ok(())
    }
}
