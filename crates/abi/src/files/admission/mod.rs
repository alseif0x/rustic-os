// SPDX-License-Identifier: Apache-2.0
//! Durable preparation and bounded explicit scheduling, separate from service-v1.
mod activity;
mod identity;
mod observation;
mod observation_v2;
mod status;
pub use activity::{Activity, ActivityPhase};
pub use identity::AdmissionId;
pub use observation::{OBSERVATION_VERSION, Observation};
pub use observation_v2::{OBSERVATION_V2, ObservationV2, PreventionReason};
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
/// Enqueue an already durable admission under current execution authority.
pub const SCHEDULE: u8 = 59;
/// Read one coherent live-or-retained observation using the stable admission ID.
pub const OBSERVE: u8 = 60;

pub const fn live(op: u8) -> bool {
    matches!(op, ACTIVITY | REQUEST_CANCEL)
}

pub const fn controlled(op: u8) -> bool {
    matches!(op, ACCEPT | EXECUTE | CANCEL)
}
