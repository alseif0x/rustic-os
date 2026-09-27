// SPDX-License-Identifier: Apache-2.0
//! Bounded transport and authority mechanisms; independent of processes/CPU.
mod broker;
mod channel;
mod handles;
mod message;
pub use broker::Broker;
pub use message::Message;
pub use rustic_abi::ipc::{ALL, Error, MAX_MESSAGE, READ, TRANSFER, WRITE};

impl From<crate::handles::Error> for Error {
    fn from(value: crate::handles::Error) -> Self {
        match value {
            crate::handles::Error::Handle => Self::Handle,
            crate::handles::Error::Denied => Self::Denied,
            crate::handles::Error::Quota => Self::Quota,
        }
    }
}

pub const CHANNELS: usize = 24;
pub const ENDPOINT_HANDLES: usize = 64;
pub const HANDLES_PER_OWNER: usize = 24;
