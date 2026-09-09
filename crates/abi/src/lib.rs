// SPDX-License-Identifier: Apache-2.0
//! Integer/wire contracts shared by kernel and future SDK; no implementation dependency.
#![no_std]
#![forbid(unsafe_code)]
pub mod application;
pub mod ipc;
pub mod process;
