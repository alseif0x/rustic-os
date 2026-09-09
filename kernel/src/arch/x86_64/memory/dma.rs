// SPDX-License-Identifier: Apache-2.0
//! Owned coherent R0 RAM. Raw volatile access never creates references to DMA bytes.
use super::{Error, Memory};
use core::marker::PhantomData;
pub(crate) struct DmaRegion {
    physical: u64,
    virtual_base: u64,
    pages: usize,
    _local: PhantomData<*mut ()>,
}
impl Memory {
    pub(crate) fn allocate_dma(&mut self, pages: usize) -> Result<DmaRegion, Error> {
        let physical = self.physical.frames.allocate_contiguous(pages)?;
        let virtual_base = self.physical.hhdm + physical;
        // SAFETY: Newly allocated contiguous RAM, wholly mapped RW/NX in HHDM.
        // No device knows these addresses yet; no references or live aliases exist.
        unsafe {
            (virtual_base as *mut u8).write_bytes(0, pages * 4096);
        }
        Ok(DmaRegion {
            physical,
            virtual_base,
            pages,
            _local: PhantomData,
        })
    }
    /// Caller must confirm reset completion; CPU/device may no longer access the region.
    pub(crate) unsafe fn release_dma(&mut self, region: DmaRegion) {
        for page in 0..region.pages {
            self.physical
                .release(region.physical + page as u64 * 4096)
                .expect("owned DMA frame");
        }
    }
}
impl DmaRegion {
    pub(crate) fn physical(&self, offset: usize) -> u64 {
        assert!(offset < self.pages * 4096);
        self.physical + offset as u64
    }
    pub(crate) fn read(&self, offset: usize, width: usize) -> u64 {
        assert!(
            [1, 2, 4, 8].contains(&width)
                && offset.is_multiple_of(width)
                && offset + width <= self.pages * 4096
        );
        let pointer = (self.virtual_base + offset as u64) as *const u8;
        // SAFETY: Checked aligned width in owned resident coherent DMA RAM.
        // Device may mutate it; volatile scalar loads create no references.
        unsafe {
            match width {
                1 => pointer.read_volatile() as u64,
                2 => pointer.cast::<u16>().read_volatile() as u64,
                4 => pointer.cast::<u32>().read_volatile() as u64,
                _ => pointer.cast::<u64>().read_volatile(),
            }
        }
    }
    pub(crate) fn write(&mut self, offset: usize, width: usize, value: u64) {
        assert!(
            [1, 2, 4, 8].contains(&width)
                && offset.is_multiple_of(width)
                && offset + width <= self.pages * 4096
        );
        let pointer = (self.virtual_base + offset as u64) as *mut u8;
        // SAFETY: Checked aligned width in owned coherent RAM. The queue owner
        // writes only driver-owned fields or buffers not currently offered to DMA.
        unsafe {
            match width {
                1 => pointer.write_volatile(value as u8),
                2 => pointer.cast::<u16>().write_volatile(value as u16),
                4 => pointer.cast::<u32>().write_volatile(value as u32),
                _ => pointer.cast::<u64>().write_volatile(value),
            }
        }
    }
}
