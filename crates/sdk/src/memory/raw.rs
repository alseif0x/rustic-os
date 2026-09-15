// SPDX-License-Identifier: Apache-2.0
//! Typed user-memory syscalls over the single `arch` instruction boundary.
//!
//! These calls take and return integers only: no pointer is handed to the
//! kernel and nothing is retained after the call. They perform no allocation
//! and hold no state; ownership of a mapped run belongs to the caller, and to
//! [`super::Heap`] when one is used.
use crate::arch;
use rustic_abi::memory as abi;
use rustic_abi::runtime::Error;

/// Kernel policy values a guest reads instead of assuming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Query {
    /// Pages this process currently holds.
    MappedPages,
    /// Pages one process may hold at once.
    PageLimit,
    /// First address of the heap window.
    WindowBase,
    /// Size of the heap window, in pages.
    WindowPages,
}

impl Query {
    const fn selector(self) -> u64 {
        match self {
            Self::MappedPages => abi::MAPPED_PAGES,
            Self::PageLimit => abi::PAGE_LIMIT,
            Self::WindowBase => abi::WINDOW_BASE,
            Self::WindowPages => abi::WINDOW_PAGES,
        }
    }
}

/// Maps `pages` zeroed, never executable pages and returns the run's base.
///
/// `address` is `None` to let the kernel choose the lowest run that fits, or an
/// explicit page-aligned, in-window address that must be completely free. The
/// kernel maps either the whole run or nothing at all.
pub fn map(address: Option<u64>, pages: u64, writable: bool) -> Result<u64, Error> {
    let flags = if writable { abi::WRITE } else { abi::READ };
    // SAFETY: MAP takes three integers; no pointer or borrow crosses the call.
    let base = Error::decode(unsafe { arch::call(abi::MAP, address.unwrap_or(0), pages, flags) })?;
    if base == 0 || base % super::PAGE != 0 || address.is_some_and(|wanted| wanted != base) {
        return Err(Error::Protocol);
    }
    Ok(base)
}

/// Releases exactly the owned run `address .. address + pages`.
///
/// A partially mapped or foreign range changes nothing and is refused.
pub fn unmap(address: u64, pages: u64) -> Result<(), Error> {
    // SAFETY: UNMAP takes three integers; no pointer or borrow crosses the call.
    if Error::decode(unsafe { arch::call(abi::UNMAP, address, pages, 0) })? != 0 {
        return Err(Error::Protocol);
    }
    Ok(())
}

pub fn query(what: Query) -> Result<u64, Error> {
    // SAFETY: QUERY takes one integer selector and two reserved zero words.
    Error::decode(unsafe { arch::call(abi::QUERY, what.selector(), 0, 0) })
}
