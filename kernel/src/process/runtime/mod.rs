// SPDX-License-Identifier: Apache-2.0
//! Guest process composition. Validation/policy stay in the no-unsafe library.
#[cfg(feature = "sdk-test")]
mod application;
mod block;
#[cfg(feature = "sdk-test")]
mod block_control;
mod error;
mod ipc_control;
mod loader;
mod manager;
#[cfg(feature = "sdk-test")]
mod native;
#[cfg(feature = "sdk-test")]
pub(crate) use native::run as terminal;
mod record;
mod syscall;
mod tests;
mod waiters;
use error::Error;
pub(crate) use tests::verify;
#[cfg(feature = "sdk-test")]
pub(crate) use tests::verify_block;
