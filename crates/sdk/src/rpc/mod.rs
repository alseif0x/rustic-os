// SPDX-License-Identifier: Apache-2.0
//! Bounded correlated exchanges; native clients can poll without blocking control.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod client;
pub mod state;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use client::Rpc;
