// SPDX-License-Identifier: Apache-2.0
//! Per-process heap window bookkeeping: which pages of the window a process owns.
//!
//! Pure page accounting. It creates no mapping, touches no frame and holds no
//! pointer; the architecture layer maps exactly the runs reserved here. The
//! window sits between the image at `0x40_0000` and the stack guard, so a heap
//! page never collides with a loaded segment or with the user stack.
//!
//! These values are kernel policy, not ABI. A guest obtains them through the
//! `QUERY` call of `rustic_abi::memory` instead of assuming them.
use crate::memory::PAGE_SIZE;

/// First address of the per-process heap window.
pub const WINDOW_BASE: u64 = 0x1000_0000;
/// Size of the window in pages; it bounds the addresses a process may ask for.
pub const WINDOW_PAGES: u64 = 128;
/// Pages one process may hold mapped at once; below `WINDOW_PAGES` on purpose,
/// so an address can be refused for lack of a run while budget remains.
pub const PROCESS_PAGES: u64 = 64;
/// First address after the window.
pub const WINDOW_END: u64 = WINDOW_BASE + WINDOW_PAGES * PAGE_SIZE;

const WORDS: usize = (WINDOW_PAGES as usize).div_ceil(u64::BITS as usize);

/// Failure classification of the window itself; the syscall layer maps each
/// variant onto the shared `rustic_abi::runtime::Error` encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Zero pages, or more pages than one process may ever hold.
    Size,
    /// Misaligned address, a range leaving the window, or an explicit range
    /// that is not completely free; also a request with no run that fits.
    Address,
    /// A release that does not name a completely owned range.
    Invalid,
    /// The per-process budget would be exceeded.
    Full,
}

/// Bitmap of the window pages owned by one process.
#[derive(Debug, Default)]
pub struct Region {
    pages: [u64; WORDS],
    used: u64,
}

/// Page index of `address` inside the window, rejecting shape errors first.
fn index_of(address: u64, pages: u64) -> Result<u64, Error> {
    if !address.is_multiple_of(PAGE_SIZE) || address < WINDOW_BASE {
        return Err(Error::Address);
    }
    let index = (address - WINDOW_BASE) / PAGE_SIZE;
    if index
        .checked_add(pages)
        .is_none_or(|end| end > WINDOW_PAGES)
    {
        return Err(Error::Address);
    }
    Ok(index)
}

impl Region {
    pub const fn new() -> Self {
        Self {
            pages: [0; WORDS],
            used: 0,
        }
    }

    /// Pages currently reserved by this process.
    pub fn used(&self) -> u64 {
        self.used
    }

    /// True only for a page-aligned range fully inside the window and fully owned.
    pub fn owns(&self, address: u64, pages: u64) -> bool {
        pages != 0
            && index_of(address, pages)
                .is_ok_and(|index| (index..index + pages).all(|page| self.taken(page)))
    }

    /// Reserve `pages` contiguous pages, either at `address` or at the lowest
    /// free run. Nothing changes when the request is refused.
    ///
    /// Order of classification: size, address shape, budget, availability.
    pub fn reserve(&mut self, address: Option<u64>, pages: u64) -> Result<u64, Error> {
        if pages == 0 || pages > PROCESS_PAGES {
            return Err(Error::Size);
        }
        let requested = match address {
            Some(address) => Some(index_of(address, pages)?),
            None => None,
        };
        if self.used + pages > PROCESS_PAGES {
            return Err(Error::Full);
        }
        let index = match requested {
            Some(index) if !self.free(index, pages) => return Err(Error::Address),
            Some(index) => index,
            // Fragmentation without an explicit address is an address failure:
            // the budget still allows the pages, no run holds them together.
            None => self.lowest_fit(pages).ok_or(Error::Address)?,
        };
        for page in index..index + pages {
            self.mark(page, true);
        }
        self.used += pages;
        Ok(WINDOW_BASE + index * PAGE_SIZE)
    }

    /// Release exactly one owned range. A partial or repeated release changes
    /// nothing and is reported as `Invalid`.
    pub fn release(&mut self, address: u64, pages: u64) -> Result<(), Error> {
        if pages == 0 {
            return Err(Error::Size);
        }
        let index = index_of(address, pages)?;
        if !self.owns(address, pages) {
            return Err(Error::Invalid);
        }
        for page in index..index + pages {
            self.mark(page, false);
        }
        self.used -= pages;
        Ok(())
    }

    fn taken(&self, index: u64) -> bool {
        self.pages[(index / 64) as usize] & (1 << (index % 64)) != 0
    }

    fn mark(&mut self, index: u64, owned: bool) {
        let bit = 1 << (index % 64);
        let word = &mut self.pages[(index / 64) as usize];
        if owned {
            *word |= bit;
        } else {
            *word &= !bit;
        }
    }

    fn free(&self, index: u64, pages: u64) -> bool {
        (index..index + pages).all(|page| !self.taken(page))
    }

    fn lowest_fit(&self, pages: u64) -> Option<u64> {
        (0..=WINDOW_PAGES.checked_sub(pages)?).find(|index| self.free(*index, pages))
    }
}
