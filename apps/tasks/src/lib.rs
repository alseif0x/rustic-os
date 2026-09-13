// SPDX-License-Identifier: Apache-2.0
//! Pure task edit planning shared by the native application and host tests.
#![no_std]
#![forbid(unsafe_code)]

#[cfg(test)]
extern crate std;

mod edit;

pub use edit::{Command, Error, Planned, plan};
