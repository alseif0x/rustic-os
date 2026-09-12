// SPDX-License-Identifier: Apache-2.0
//! Explicit single-method lifecycle selection. Discovery carries no authority.
mod descriptor;
mod request;
mod reviewed;
pub use descriptor::{Descriptor, Limits};
use request::method;
pub use request::{decode_request, request};

pub const DESCRIBE: u8 = 62;
pub const VERSION: u64 = 2;
/// Native stable-admission inspection / minimal cancellation, not the v1 lifecycle.
pub const PROFILE: u32 = 1;
