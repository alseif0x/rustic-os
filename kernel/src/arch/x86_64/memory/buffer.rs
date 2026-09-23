// SPDX-License-Identifier: Apache-2.0
//! Bounded kernel-owned bytes backed by a contiguous run of frames.
use super::{Error, Memory};
use rustic_kernel::memory::PAGE_SIZE;

pub(crate) const MAX_KERNEL_BUFFER_PAGES: usize = 128;

pub(crate) struct KernelBuffer {
    physical: u64,
    virtual_base: u64,
    pages: usize,
    length: usize,
}

impl Memory {
    pub(crate) fn allocate_kernel_buffer(&mut self, length: usize) -> Result<KernelBuffer, Error> {
        let pages = length.div_ceil(PAGE_SIZE as usize);
        if pages == 0 || pages > MAX_KERNEL_BUFFER_PAGES {
            return Err(Error::InvalidAddress);
        }
        let physical = self
            .physical
            .frames
            .allocate_contiguous_bounded(pages, MAX_KERNEL_BUFFER_PAGES)?;
        let virtual_base = self.physical.hhdm + physical;
        // SAFETY: These contiguous frames are newly allocated, HHDM mapped, and
        // have no aliases or device access. The range covers exactly every page.
        unsafe {
            (virtual_base as *mut u8).write_bytes(0, pages * PAGE_SIZE as usize);
        }
        Ok(KernelBuffer {
            physical,
            virtual_base,
            pages,
            length,
        })
    }

    /// Erases the complete allocation before returning every owned frame.
    pub(crate) fn release_kernel_buffer(&mut self, buffer: KernelBuffer) {
        // SAFETY: `buffer` uniquely owns these frames; no reference or device
        // access escapes this type, and the buffer is consumed by this method.
        unsafe {
            (buffer.virtual_base as *mut u8).write_bytes(0, buffer.pages * PAGE_SIZE as usize);
        }
        for page in 0..buffer.pages {
            self.physical
                .release(buffer.physical + page as u64 * PAGE_SIZE)
                .expect("owned kernel buffer frame");
        }
    }
}

impl KernelBuffer {
    pub(crate) fn write(&mut self, offset: usize, bytes: &[u8]) -> Result<(), Error> {
        if bytes.is_empty()
            || offset
                .checked_add(bytes.len())
                .is_none_or(|end| end > self.length)
        {
            return Err(Error::InvalidAddress);
        }
        // SAFETY: The checked destination interval is within uniquely owned
        // kernel RAM. The caller supplies a disjoint bounded copy buffer.
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                (self.virtual_base as *mut u8).add(offset),
                bytes.len(),
            );
        }
        Ok(())
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        // SAFETY: The allocation is contiguous and remains owned by `self` for
        // the returned borrow. Mutations require `&mut self`, so no alias writes.
        unsafe { core::slice::from_raw_parts(self.virtual_base as *const u8, self.length) }
    }
}
