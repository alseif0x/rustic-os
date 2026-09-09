// SPDX-License-Identifier: Apache-2.0
//! Freestanding kernel mechanisms, independent of firmware and host tooling.
#![no_std]
#![forbid(unsafe_code)]

pub mod boot;
pub mod ipc;
pub mod memory;
pub mod process;
pub mod time;
