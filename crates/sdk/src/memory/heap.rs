// SPDX-License-Identifier: Apache-2.0
//! A guest-owned run of mapped pages with a bounded allocator inside it.
//!
//! The heap owns exactly the pages it mapped and releases them when it is
//! dropped. It never relocates: growing maps the pages directly after the
//! current end, and a kernel refusal of that address is returned unchanged.
//! Frames remain the kernel's concern, mapping belongs to [`super::raw`], block
//! policy to [`super::allocator`], and the data in a block to the caller.
use super::allocator::{Allocator, Block, Stats};
use super::raw;
use super::{Error, PAGE};
use core::ptr::NonNull;
use rustic_abi::runtime;

pub struct Heap {
    base: u64,
    pages: u64,
    writable: bool,
    /// Absent for a read-only reservation, which cannot hold bookkeeping.
    allocator: Option<Allocator<'static>>,
}

impl Heap {
    /// Maps one run of `pages` zeroed pages chosen by the kernel.
    pub fn reserve(pages: u64, writable: bool) -> Result<Self, Error> {
        let base = raw::map(None, pages, writable)?;
        let mut heap = Self {
            base,
            pages,
            writable,
            allocator: None,
        };
        if writable {
            let bytes = heap.byte_length()?;
            let address = usize::try_from(base).map_err(|_| Error::Map(runtime::Error::Address))?;
            let pointer = NonNull::new(core::ptr::with_exposed_provenance_mut::<u8>(address))
                .ok_or(Error::Map(runtime::Error::Address))?;
            // SAFETY: the kernel has just granted exactly `bytes` writable,
            // zeroed and therefore initialized bytes at `base` to this process
            // alone. `heap` is the only owner of that run, keeps it mapped until
            // its `Drop`, and no second allocator is built over it. A failure
            // below drops `heap`, which unmaps the run again.
            let allocator = unsafe { Allocator::from_raw(pointer, bytes) }?;
            heap.allocator = Some(allocator);
        }
        Ok(heap)
    }

    /// First address of the run; stable for the heap's whole life.
    pub fn base(&self) -> u64 {
        self.base
    }

    /// Pages currently mapped by this heap.
    pub fn pages(&self) -> u64 {
        self.pages
    }

    /// Whether the run was mapped writable and therefore carries an allocator.
    pub fn writable(&self) -> bool {
        self.writable
    }

    pub fn stats(&self) -> Result<Stats, Error> {
        Ok(self.allocator.as_ref().ok_or(Error::ReadOnly)?.stats())
    }

    /// Maps `pages` more pages exactly after the current end.
    ///
    /// The heap never moves: if the kernel refuses that address, the error is
    /// returned and the existing run is untouched.
    pub fn grow(&mut self, pages: u64) -> Result<(), Error> {
        let end = self
            .base
            .checked_add(self.mapped_bytes()?)
            .ok_or(Error::Map(runtime::Error::Address))?;
        raw::map(Some(end), pages, self.writable)?;
        self.pages += pages;
        let extra = usize::try_from(pages * PAGE).map_err(|_| Error::Map(runtime::Error::Size))?;
        if let Some(allocator) = self.allocator.as_mut() {
            // SAFETY: the kernel has just granted exactly this run, writable and
            // zeroed, directly after the region the allocator already owns. It
            // stays mapped until this heap shrinks or drops it, and the heap is
            // its only owner.
            unsafe { allocator.extend(extra) }?;
        }
        Ok(())
    }

    /// Unmaps whole trailing pages that hold no live block, and reports how
    /// many pages were released.
    pub fn shrink_trailing(&mut self) -> Result<u64, Error> {
        let allocator = self.allocator.as_mut().ok_or(Error::ReadOnly)?;
        let capacity = allocator.capacity();
        let target = allocator.truncation_target(PAGE as usize);
        if target == capacity {
            return Ok(0);
        }
        let released = (capacity - target) as u64 / PAGE;
        // Unmapping may fail and then changes nothing, while `truncate` always
        // accepts a target that `truncation_target` produced. Doing the
        // fallible step first is therefore what keeps the mapping and the
        // accounting in step; the reverse order could strand released pages.
        raw::unmap(self.base + target as u64, released)?;
        allocator.truncate(target)?;
        self.pages -= released;
        Ok(released)
    }

    pub fn alloc(&mut self, size: usize, align: usize) -> Result<Block, Error> {
        Ok(self
            .allocator
            .as_mut()
            .ok_or(Error::ReadOnly)?
            .alloc(size, align)?)
    }

    pub fn free(&mut self, block: Block) -> Result<(), Error> {
        Ok(self
            .allocator
            .as_mut()
            .ok_or(Error::ReadOnly)?
            .free(block)?)
    }

    pub fn bytes(&self, block: &Block) -> Result<&[u8], Error> {
        Ok(self
            .allocator
            .as_ref()
            .ok_or(Error::ReadOnly)?
            .bytes(block)?)
    }

    pub fn bytes_mut(&mut self, block: &Block) -> Result<&mut [u8], Error> {
        Ok(self
            .allocator
            .as_mut()
            .ok_or(Error::ReadOnly)?
            .bytes_mut(block)?)
    }

    fn mapped_bytes(&self) -> Result<u64, Error> {
        self.pages
            .checked_mul(PAGE)
            .ok_or(Error::Map(runtime::Error::Size))
    }

    fn byte_length(&self) -> Result<usize, Error> {
        usize::try_from(self.mapped_bytes()?).map_err(|_| Error::Map(runtime::Error::Size))
    }
}

impl Drop for Heap {
    fn drop(&mut self) {
        if self.pages > 0 {
            // Process exit remains the backstop; an explicit release keeps the
            // per-process budget correct for a long-lived application.
            let _ = raw::unmap(self.base, self.pages);
        }
    }
}
