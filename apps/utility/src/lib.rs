// SPDX-License-Identifier: Apache-2.0
//! Library face of the utility application.
//!
//! The binary keeps every diagnostic that only exists to be driven by an owner.
//! What lives here is the one responsibility that is a product behaviour of its
//! own: acting as the second native semantic tasks client. Its step machine and
//! its reply encoding decide nothing about files, so they are exercised by host
//! tests, while the exchange that borrows file authority stays guest-only. The
//! compile-time build tag lives here too, so its decoding is host-tested.
#![no_std]
#![forbid(unsafe_code)]

pub mod build_tag;
pub mod tasks;
