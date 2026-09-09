// SPDX-License-Identifier: Apache-2.0
//! Guest process composition. Validation/policy stay in the no-unsafe library.
#[cfg(feature = "sdk-test")]
mod application;
mod error;
mod ipc_control;
mod loader;
mod manager;
mod record;
mod syscall;
mod tests;
use error::Error;
pub(crate) use tests::verify;
