// SPDX-License-Identifier: Apache-2.0
//! Service-v1 bounded range contract over the independent native file framing.
mod request;
mod response;
pub use request::Request;
pub use response::{Header, Info};
pub const VERSION: u16 = 1;
pub const MAX_RANGE: usize = 1024;
pub const MAX_INTEGER: u64 = (1 << 53) - 1;
