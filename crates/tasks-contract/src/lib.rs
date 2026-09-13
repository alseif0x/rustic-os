// SPDX-License-Identifier: Apache-2.0
//! Versioned task document and the private native row protocol.
//!
//! This crate contains only the product contract. It has no kernel, SDK or host
//! dependencies, so host tests exercise the same parser and row wire format
//! that the native application uses.
#![no_std]
#![forbid(unsafe_code)]

#[cfg(test)]
extern crate std;

#[cfg(feature = "tasks-acceptance")]
pub mod acceptance;
mod document;
pub mod preview;
pub mod wire;

pub use document::{Document, Error, MAX_BYTES, MAX_TASKS, MAX_TITLE, State, Task};
