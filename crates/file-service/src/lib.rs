// SPDX-License-Identifier: Apache-2.0
//! File policy and transfer state shared by native server and portable adversarial tests.
#![no_std]
#![forbid(unsafe_code)]
mod authority;
mod dispatch;
mod grants;
mod recovery;
mod reply;
mod transfer;
pub use authority::{CLIENTS, Grant};
pub use dispatch::Server;
mod validation;
