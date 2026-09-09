// SPDX-License-Identifier: Apache-2.0
use super::{BootMemory, Error, bootstrap};
use core::sync::atomic::{AtomicBool, Ordering};
use rustic_kernel::memory::{FrameAllocator, PAGE_SIZE, canonical};

pub(super) const LIMIT: u64 = 1 << 30;
const WORDS: usize = (LIMIT / PAGE_SIZE / 64) as usize;
static TAKEN: AtomicBool = AtomicBool::new(false);
static mut MANAGED: [u64; WORDS] = [0; WORDS];
static mut ALLOCATED: [u64; WORDS] = [0; WORDS];

pub(super) struct Physical {
    pub(super) frames: FrameAllocator<'static>,
    hhdm: u64,
}

impl Physical {
    pub(super) fn initialize(
        layout: BootMemory,
        entries: impl Iterator<Item = (u64, u64, bool)> + Clone,
    ) -> Result<Self, Error> {
        if layout.hhdm < 0xffff_8000_0000_0000
            || !layout.hhdm.is_multiple_of(PAGE_SIZE)
            || layout
                .hhdm
                .checked_add(LIMIT)
                .is_none_or(|end| !canonical(end))
        {
            return Err(Error::InvalidAddress);
        }
        if TAKEN.swap(true, Ordering::Relaxed) {
            return Err(Error::AlreadyInitialized);
        }
        // SAFETY: Unique bootstrap owner of disjoint static bitmaps. No allocator
        // runs in IRQ/NMI context; these references never escape the memory owner.
        let mut frames = unsafe {
            FrameAllocator::new(
                &mut *core::ptr::addr_of_mut!(MANAGED),
                &mut *core::ptr::addr_of_mut!(ALLOCATED),
            )?
        };
        frames.import(entries)?;
        frames.reserve(0, 1024 * 1024)?;
        let (start, end) = bootstrap::image_range();
        if layout.virtual_base != start
            || !layout.physical_base.is_multiple_of(PAGE_SIZE)
            || layout
                .physical_base
                .checked_add(end - start)
                .is_none_or(|end| end > LIMIT)
        {
            return Err(Error::InvalidAddress);
        }
        frames.reserve(layout.physical_base, end - start)?;
        Ok(Self {
            frames,
            hhdm: layout.hhdm,
        })
    }

    /// Addresses originate in trusted loader tables or this allocator, never users.
    fn pointer(&self, address: u64) -> *mut u64 {
        assert!(address < LIMIT && address.is_multiple_of(PAGE_SIZE));
        (self.hhdm + address) as *mut u64
    }

    pub(super) fn allocate_zeroed(&mut self) -> Result<u64, Error> {
        let frame = self.frames.allocate()?;
        // SAFETY: Newly owned full RAM page, mapped in HHDM, no live aliases or
        // references to its contents. Zero before exposing to another address space.
        unsafe {
            self.pointer(frame)
                .cast::<u8>()
                .write_bytes(0, PAGE_SIZE as usize);
        }
        Ok(frame)
    }

    pub(super) fn read(&self, table: u64, index: usize) -> u64 {
        assert!(index < 512);
        // SAFETY: Validated/trusted resident page-table RAM; index remains within
        // this page. Volatile raw access creates no Rust references to CPU tables.
        unsafe { self.pointer(table).add(index).read_volatile() }
    }

    pub(super) fn write(&mut self, table: u64, index: usize, value: u64) {
        assert!(index < 512 && self.frames.is_allocated(table));
        // SAFETY: Unique manager owns this allocated page. Aligned entry writes;
        // hardware may set A/D, but no second software writer or IRQ allocator exists.
        unsafe {
            self.pointer(table).add(index).write_volatile(value);
        }
    }

    pub(super) fn release(&mut self, frame: u64) -> Result<(), Error> {
        self.frames.release(frame).map_err(Into::into)
    }

    pub(super) fn initialize_bytes(&mut self, frame: u64, offset: usize, data: &[u8]) {
        assert!(self.frames.is_allocated(frame));
        assert!(
            offset
                .checked_add(data.len())
                .is_some_and(|end| end <= PAGE_SIZE as usize)
        );
        // SAFETY: Owner holds an allocated page in an inactive user root. No user
        // runs, IRQ/DMA never access these bytes, no data references escape,
        // source is disjoint kernel bytes, and the checked interval fits.
        unsafe {
            core::ptr::copy_nonoverlapping(
                data.as_ptr(),
                self.pointer(frame).cast::<u8>().add(offset),
                data.len(),
            )
        }
    }

    pub(super) fn empty(&self, table: u64) -> bool {
        (0..512).all(|index| self.read(table, index) == 0)
    }
    pub(super) fn read_bytes(&self, frame: u64, offset: usize, data: &mut [u8]) {
        assert!(self.frames.is_allocated(frame));
        assert!(
            offset
                .checked_add(data.len())
                .is_some_and(|end| end <= PAGE_SIZE as usize)
        );
        // SAFETY: Quiescent owned user page, validated full interval, resident HHDM;
        // output is disjoint kernel storage. No user or DMA can mutate it here.
        unsafe {
            core::ptr::copy_nonoverlapping(
                self.pointer(frame).cast::<u8>().add(offset),
                data.as_mut_ptr(),
                data.len(),
            )
        }
    }
}
