// SPDX-License-Identifier: Apache-2.0
//! Explicit large-workspace wire profile; codecs do not advertise service support.
//!
//! The 64-byte transport and identity types stay unchanged. Profile markers make
//! old and new operation messages mutually rejecting even for small files.
mod receipt;
mod request;
pub use receipt::Operation;
pub use request::{Lookup, Replacement};

pub const PROFILE: u32 = 2;
pub const MAX_FILE_BYTES: u32 = 512 * 1024;
pub const RECEIPT_BYTES: usize = super::operation::RECEIPT_BYTES;
/// Retained operation records a profile-2 (format-7) volume holds. The
/// V7 file service checks at compile time that the volume owner agrees.
pub const RETAINED_RECORDS: u32 = 8;
