// SPDX-License-Identifier: Apache-2.0
#![no_std]

#[cfg(test)]
extern crate std;

mod poller;
pub use poller::{Poller, Requests};
