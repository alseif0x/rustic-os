// SPDX-License-Identifier: Apache-2.0
//! Owned native block requests; user memory is borrowed only during immediate copying.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod client;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use client::Device;
pub use rustic_abi::block::{Completion, Effect, Error, Geometry, Operation, SECTOR, Status};
