// SPDX-License-Identifier: Apache-2.0
//! Dynamic user heap extension 1: additive INT 0x80 numbers, flags and selectors.
//!
//! The call numbers, the flag encoding and the error encoding are the fixed ABI.
//! The window address, the window size and the per-process page limit are kernel
//! policy: a guest reads them with [`QUERY`] and never assumes the values below.
//! Errors reuse the [`crate::runtime::Error`] encoding of the native extension.
//!
//! Argument words follow `docs/PROCESS-ABI.md`: RAX is the call number, RDI the
//! first word, RSI the second and RDX the third; the result is returned in RAX.
//!
//! | RAX | Name | RDI | RSI | RDX | Result |
//! | --- | --- | --- | --- | --- | --- |
//! | 21 | [`MAP`] | Address or 0 | Pages | Flags | Base address of the run |
//! | 22 | [`UNMAP`] | Address | Pages | 0 | 0 |
//! | 23 | [`QUERY`] | Selector | Reserved | Reserved | Selected policy value |
//!
//! `MAP` with RDI = 0 asks the kernel for the lowest free run that fits inside
//! the window; an explicit address must be page aligned, inside the window and
//! completely unmapped. Every page is granted zeroed and is never executable.
//! `UNMAP` releases exactly one owned run and changes nothing when it fails.
//!
//! The reserved words are deliberately asymmetric: `UNMAP` requires RDX to be
//! zero and reports `Invalid` for any other value, so a future third argument
//! stays available, while `QUERY` ignores RSI and RDX entirely. A guest should
//! pass zero in every reserved word regardless.

/// Extension version of this contract; independent of `process::VERSION`.
pub const VERSION: u32 = 1;

pub const MAP: u64 = 21;
pub const UNMAP: u64 = 22;
pub const QUERY: u64 = 23;

/// Readable pages. Absence of every flag bit; pages are never executable.
pub const READ: u64 = 0;
/// Readable and writable pages.
pub const WRITE: u64 = 1;

/// Pages currently mapped by the caller.
pub const MAPPED_PAGES: u64 = 0;
/// Pages a single process may hold at once.
pub const PAGE_LIMIT: u64 = 1;
/// First address of the heap window.
pub const WINDOW_BASE: u64 = 2;
/// Size of the heap window in pages.
pub const WINDOW_PAGES: u64 = 3;

/// Only the documented flag bits are accepted; any other bit is `Invalid`.
pub const fn flags_valid(flags: u64) -> bool {
    flags & !WRITE == 0
}

/// Selectors outside the documented set are `Invalid`.
pub const fn selector_valid(selector: u64) -> bool {
    selector <= WINDOW_PAGES
}
