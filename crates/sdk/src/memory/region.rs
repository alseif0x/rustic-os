// SPDX-License-Identifier: Apache-2.0
//! The only raw-memory access behind the SDK allocator.
//!
//! A `Region` is a bounds-checked view over one contiguous, exclusively owned
//! and writable byte range. Every accessor validates its offset and length, so
//! the block policy in [`super::allocator`] needs no `unsafe` at all.
//!
//! Invariants established once, at construction, and relied on afterwards:
//! * `base` is non-null and aligned to [`ALIGN`];
//! * `base .. base + len` is one allocated object that the region owns
//!   exclusively for `'a`, is writable and stays mapped for that whole period;
//! * every byte in that range is initialized (a host slice, or a page the
//!   kernel granted zeroed), so integer reads never observe uninitialized data;
//! * `len` is a multiple of [`ALIGN`], and no other `Region` aliases the range.
use core::marker::PhantomData;
use core::ptr::NonNull;

/// Alignment of every block start and block size inside a region.
pub(super) const ALIGN: usize = core::mem::align_of::<Header>();
/// Bookkeeping bytes that precede the payload of every block.
pub(super) const HEADER: usize = core::mem::size_of::<Header>();

/// In-region bookkeeping: blocks tile the region and are linked by size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub(super) struct Header {
    /// Total bytes of this block, header included.
    pub(super) size: usize,
    /// Total bytes of the preceding block, `0` for the first block.
    pub(super) prev: usize,
    /// Free or used marker owned by the allocator.
    pub(super) state: usize,
}

pub(super) struct Region<'a> {
    base: NonNull<u8>,
    len: usize,
    owned: PhantomData<&'a mut [u8]>,
}

impl<'a> Region<'a> {
    /// Borrows a caller-provided buffer; the usable range is trimmed to the
    /// first [`ALIGN`]-aligned address and a whole number of [`ALIGN`] units.
    pub(super) fn new(bytes: &'a mut [u8]) -> Option<Self> {
        let total = bytes.len();
        let start = bytes.as_mut_ptr();
        let padding = start.addr().next_multiple_of(ALIGN) - start.addr();
        let len = total.checked_sub(padding)? & !(ALIGN - 1);
        if len == 0 {
            return None;
        }
        // SAFETY: `padding <= total`, so the offset stays inside the same
        // borrowed allocation and the resulting pointer is non-null.
        let base = unsafe { NonNull::new_unchecked(start.add(padding)) };
        Some(Self {
            base,
            len,
            owned: PhantomData,
        })
    }

    /// # Safety
    /// `base` must point at `len` writable bytes of one allocated object that
    /// the caller owns exclusively for `'a`, keeps mapped for that whole period
    /// and never exposes to a second `Region`. Every byte must be initialized.
    pub(super) unsafe fn from_raw(base: NonNull<u8>, len: usize) -> Option<Self> {
        if !base.addr().get().is_multiple_of(ALIGN) {
            return None;
        }
        let len = len & !(ALIGN - 1);
        if len == 0 {
            return None;
        }
        Some(Self {
            base,
            len,
            owned: PhantomData,
        })
    }

    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn address(&self) -> usize {
        self.base.addr().get()
    }

    fn holds(&self, offset: usize, len: usize) -> bool {
        offset.checked_add(len).is_some_and(|end| end <= self.len)
    }

    pub(super) fn header(&self, offset: usize) -> Option<Header> {
        if !offset.is_multiple_of(ALIGN) || !self.holds(offset, HEADER) {
            return None;
        }
        // SAFETY: the offset is inside the owned range and aligned for `Header`,
        // whose fields are plain integers over initialized bytes; `&self`
        // excludes a concurrent write through this unique region.
        Some(unsafe { self.base.as_ptr().add(offset).cast::<Header>().read() })
    }

    pub(super) fn set_header(&mut self, offset: usize, header: Header) -> bool {
        if !offset.is_multiple_of(ALIGN) || !self.holds(offset, HEADER) {
            return false;
        }
        // SAFETY: as in `header`, plus `&mut self` proving there is no live
        // borrow of these bytes while they are overwritten.
        unsafe {
            self.base
                .as_ptr()
                .add(offset)
                .cast::<Header>()
                .write(header);
        }
        true
    }

    pub(super) fn pointer(&self, offset: usize) -> Option<NonNull<u8>> {
        if offset > self.len {
            return None;
        }
        // SAFETY: `offset <= len` keeps the result inside the owned object, and
        // an in-bounds offset from a non-null base is itself non-null.
        Some(unsafe { NonNull::new_unchecked(self.base.as_ptr().add(offset)) })
    }

    pub(super) fn bytes(&self, offset: usize, len: usize) -> Option<&[u8]> {
        if !self.holds(offset, len) {
            return None;
        }
        // SAFETY: the range lies inside the initialized owned object and the
        // returned borrow inherits `&self`, so no write can alias it.
        Some(unsafe { core::slice::from_raw_parts(self.base.as_ptr().add(offset), len) })
    }

    pub(super) fn bytes_mut(&mut self, offset: usize, len: usize) -> Option<&mut [u8]> {
        if !self.holds(offset, len) {
            return None;
        }
        // SAFETY: as in `bytes`; `&mut self` proves the exclusive borrow that
        // the returned slice requires.
        Some(unsafe { core::slice::from_raw_parts_mut(self.base.as_ptr().add(offset), len) })
    }

    /// # Safety
    /// `extra` bytes directly after the current end must satisfy every region
    /// invariant: owned, writable, initialized and mapped for `'a`.
    pub(super) unsafe fn grow(&mut self, extra: usize) {
        self.len += extra;
    }

    pub(super) fn shrink(&mut self, len: usize) -> bool {
        if len > self.len {
            return false;
        }
        self.len = len;
        true
    }
}
