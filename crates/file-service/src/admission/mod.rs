// SPDX-License-Identifier: Apache-2.0
//! Typed service integration before accepted/result IPC. Admission is not authority.
mod authority;
mod control;
mod transition;

/// Bound by a trusted transport to its live endpoint, authenticated sender and
/// context. The model/request payload must never supply the peer or slot binding.
#[derive(Clone, Copy)]
pub struct Caller {
    pub slot: usize,
    pub peer: u64,
    pub context: u32,
}
