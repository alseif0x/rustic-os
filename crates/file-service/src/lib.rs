// SPDX-License-Identifier: Apache-2.0
//! File policy and transfer state shared by native server and portable adversarial tests.
#![no_std]
#![forbid(unsafe_code)]
mod admission;
mod authority;
mod capabilities;
mod negotiation;
pub use admission::{ActiveExecution, Caller, ExecutionQueue};
mod clients;
pub use clients::Clients;
mod disk;
mod dispatch;
mod grants;
mod operations;
mod read;
mod recovery;
mod reply;
mod transfer;
mod v7_read;
pub use authority::{CLIENTS, Grant};
pub use dispatch::Server;
pub use v7_read::{READ_CLIENTS7, ReadGrant7, ReadServer7};
mod validation;
