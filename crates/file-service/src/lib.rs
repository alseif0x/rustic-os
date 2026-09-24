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
mod v7;
pub use authority::{CLIENTS, Grant};
pub use dispatch::Server;
pub use v7::{CLIENTS7, Grant7, GrantRequest7, Maintenance7, READ_ONLY7, Server7, TRACKED_WRITE7};
mod validation;
