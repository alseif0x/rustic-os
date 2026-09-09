// SPDX-License-Identifier: Apache-2.0
//! Allocation-free deadlines. Hardware supplies monotonic ticks; no wall clock.
mod deadline;
mod waits;

pub use deadline::{Deadline, Overflow, ticks_to_nanos};
pub use waits::{WaitError, WaitSet};
