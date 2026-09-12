// SPDX-License-Identifier: Apache-2.0
//! Service-v2 lifecycle for retained admissions. Independent of storage encoding.
mod cancellation;
mod observation;
pub use cancellation::{CancelAck, Disposition};
pub use observation::{Failure, Operation, State};

pub const VERSION: u32 = 2;
pub const CANCEL: u8 = 61;
