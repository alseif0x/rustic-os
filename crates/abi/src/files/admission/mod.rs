// SPDX-License-Identifier: Apache-2.0
//! Explicitly scheduled durable admissions. This is not the service-v1 async profile.
mod activity;
mod identity;
mod status;
pub use activity::{Activity, ActivityPhase};
pub use identity::AdmissionId;
pub use status::{State, Status};
pub const OPEN: u8 = 48;
pub const CHUNK: u8 = 49;
pub const ACCEPT: u8 = 50;
pub const ABORT: u8 = 51;
pub const GET: u8 = 52;
pub const RETRY: u8 = 53;
pub const EXECUTE: u8 = 54;
pub const CANCEL: u8 = 55;
pub const ACTIVITY: u8 = 56;
pub const REQUEST_CANCEL: u8 = 57;

pub const fn live(op: u8) -> bool {
    matches!(op, ACTIVITY | REQUEST_CANCEL)
}

pub const fn controlled(op: u8) -> bool {
    matches!(op, ACCEPT | EXECUTE | CANCEL)
}
