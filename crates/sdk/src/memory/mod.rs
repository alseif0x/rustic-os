// SPDX-License-Identifier: Apache-2.0
//! Explicit, bounded user memory. There is no global allocator and no `alloc`.
//!
//! Responsibilities stay separated: the kernel owns frames and page tables,
//! [`raw`] is the typed syscall boundary for mapping, [`allocator`] is pure
//! block policy inside one region, and [`Heap`] is the guest object that owns a
//! mapped run and delegates to both. Application data belongs to the caller.
//!
//! Window base, window size and the per-process page limit are kernel policy
//! and are read at runtime with [`raw::query`]; nothing here hardcodes them.
pub mod allocator;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod heap;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub mod raw;
mod region;

pub use allocator::{AllocError, Allocator, Block, Stats};
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use heap::Heap;
use rustic_abi::runtime;

/// Bytes in one page of the heap window; the extension counts in these units.
pub const PAGE: u64 = 4096;

/// A heap operation fails either at the kernel boundary or in block policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The kernel refused a map, unmap or query.
    Map(runtime::Error),
    /// The region could not satisfy the request.
    Alloc(AllocError),
    /// The heap was mapped read-only, so it carries no allocator.
    ReadOnly,
}

impl From<runtime::Error> for Error {
    fn from(error: runtime::Error) -> Self {
        Self::Map(error)
    }
}

impl From<AllocError> for Error {
    fn from(error: AllocError) -> Self {
        Self::Alloc(error)
    }
}
