// SPDX-License-Identifier: Apache-2.0
//! Durable admissions with explicit public execution. Admission is not authority.
mod active;
mod authority;
mod observation;
mod scheduling;
mod scope;
pub use scheduling::ExecutionQueue;
mod execution;
mod prevention;
pub use active::ActiveExecution;
mod cancellation;
mod control;
mod transition;
mod wire;

/// Bound by a trusted transport to its live endpoint, authenticated sender and
/// context. The model/request payload must never supply the peer or slot binding.
#[derive(Clone, Copy)]
pub struct Caller {
    pub slot: usize,
    pub peer: u64,
    pub context: u32,
}
