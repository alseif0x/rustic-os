// SPDX-License-Identifier: Apache-2.0
//! Bounded transport and authority mechanisms; independent of processes/CPU.
mod broker;
mod channel;
mod handles;
mod message;
pub use broker::Broker;
pub use message::Message;
pub use rustic_abi::ipc::{ALL, Error, MAX_MESSAGE, READ, TRANSFER, WRITE};
