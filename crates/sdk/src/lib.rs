// SPDX-License-Identifier: Apache-2.0
//! Allocation-free native SDK. Runtime entry points exist only on the guest target.
#![no_std]
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod arch;
pub mod error;
pub mod ipc;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub mod process;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod startup;
pub use error::Error;
pub use rustic_abi as abi;
