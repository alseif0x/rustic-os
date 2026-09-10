// SPDX-License-Identifier: Apache-2.0
//! Bounded native block extension; independent from IPC and service JSON schemas.
mod completion;
mod error;
mod geometry;
mod request;
mod wire;
pub use completion::{Completion, Effect, Status};
pub use error::Error;
pub use geometry::Geometry;
pub use request::{Operation, Request};
pub const VERSION: u16 = 1;
pub const INFO: u64 = 9;
pub const SUBMIT: u64 = 10;
pub const RESULT: u64 = 11;
pub const WAIT: u64 = 12;
pub const CANCEL: u64 = 13;
pub const CLOSE: u64 = 14;
pub const READ: u8 = 1;
pub const WRITE: u8 = 2;
pub const FLUSH: u8 = 4;
pub const ALL: u8 = READ | WRITE | FLUSH;
pub const SECTOR: usize = 512;
pub const REQUEST_BYTES: usize = 32;
pub const GEOMETRY_BYTES: usize = 32;
pub const RESULT_BYTES: usize = 32 + SECTOR;
pub const MAX_ID: u64 = (1 << 56) - 1;
