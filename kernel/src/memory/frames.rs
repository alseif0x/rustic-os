// SPDX-License-Identifier: Apache-2.0
use super::PAGE_SIZE;
use crate::boot::validate_map;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    InvalidMap,
    InvalidStorage,
    AddressLimit,
    InvalidRange,
    NotManaged,
    NotAllocated,
    InUse,
    Exhausted,
}

/// Bitmaps belong to one owner. This records physical numbers, not Rust allocations.
pub struct FrameAllocator<'a> {
    managed: &'a mut [u64],
    allocated: &'a mut [u64],
    free: usize,
    total: usize,
    cursor: usize,
}

impl<'a> FrameAllocator<'a> {
    pub fn new(managed: &'a mut [u64], allocated: &'a mut [u64]) -> Result<Self, FrameError> {
        if managed.is_empty() || managed.len() != allocated.len() {
            return Err(FrameError::InvalidStorage);
        }
        managed.fill(0);
        allocated.fill(0);
        Ok(Self {
            managed,
            allocated,
            free: 0,
            total: 0,
            cursor: 0,
        })
    }

    /// Import only whole usable pages. The iterator is repeatable for full validation
    /// before mutation; bootloader-reclaimable/unknown types must be non-usable.
    pub fn import(
        &mut self,
        entries: impl Iterator<Item = (u64, u64, bool)> + Clone,
    ) -> Result<(), FrameError> {
        if self.total != 0 {
            return Err(FrameError::InUse);
        }
        validate_map(entries.clone()).map_err(|_| FrameError::InvalidMap)?;
        let limit = self.managed.len() as u64 * 64 * PAGE_SIZE;
        for (start, length, usable) in entries.clone() {
            if usable && start.checked_add(length).is_none_or(|end| end > limit) {
                return Err(FrameError::AddressLimit);
            }
        }
        for (start, length, usable) in entries {
            if !usable {
                continue;
            }
            let first = start.div_ceil(PAGE_SIZE);
            let end = (start + length) / PAGE_SIZE;
            for frame in first..end {
                self.managed[frame as usize / 64] |= 1 << (frame % 64);
                self.free += 1;
                self.total += 1;
            }
        }
        Ok(())
    }

    /// Outward rounding reserves every page touched by the byte range. Atomic on error.
    pub fn reserve(&mut self, start: u64, length: u64) -> Result<(), FrameError> {
        let end = start
            .checked_add(length)
            .filter(|_| length != 0)
            .ok_or(FrameError::InvalidRange)?;
        let first = start / PAGE_SIZE;
        let last = end.div_ceil(PAGE_SIZE).min(self.managed.len() as u64 * 64);
        for frame in first..last {
            if self.allocated[frame as usize / 64] & (1 << (frame % 64)) != 0 {
                return Err(FrameError::InUse);
            }
        }
        for frame in first..last {
            let word = &mut self.managed[frame as usize / 64];
            let mask = 1 << (frame % 64);
            if *word & mask != 0 {
                *word &= !mask;
                self.free -= 1;
                self.total -= 1;
            }
        }
        Ok(())
    }

    pub fn allocate(&mut self) -> Result<u64, FrameError> {
        if self.free == 0 {
            return Err(FrameError::Exhausted);
        }
        for offset in 0..self.managed.len() {
            let index = (self.cursor + offset) % self.managed.len();
            let available = self.managed[index] & !self.allocated[index];
            if available != 0 {
                let bit = available.trailing_zeros();
                self.allocated[index] |= 1 << bit;
                self.cursor = index;
                self.free -= 1;
                return Ok((index as u64 * 64 + u64::from(bit)) * PAGE_SIZE);
            }
        }
        Err(FrameError::Exhausted)
    }

    pub fn release(&mut self, address: u64) -> Result<(), FrameError> {
        let (index, mask) = self.locate(address)?;
        if self.managed[index] & mask == 0 {
            return Err(FrameError::NotManaged);
        }
        if self.allocated[index] & mask == 0 {
            return Err(FrameError::NotAllocated);
        }
        self.allocated[index] &= !mask;
        self.free += 1;
        self.cursor = index;
        Ok(())
    }

    /// Bounded contiguous DMA allocation. Search and validation precede mutation.
    pub fn allocate_contiguous(&mut self, count: usize) -> Result<u64, FrameError> {
        if count == 0 || count > 4 {
            return Err(FrameError::InvalidRange);
        }
        let mut run = 0;
        for frame in 0..self.managed.len() * 64 {
            let mask = 1u64 << (frame % 64);
            if self.managed[frame / 64] & !self.allocated[frame / 64] & mask != 0 {
                run += 1;
                if run == count {
                    let first = frame + 1 - count;
                    for owned in first..=frame {
                        self.allocated[owned / 64] |= 1 << (owned % 64);
                    }
                    self.free -= count;
                    return Ok(first as u64 * PAGE_SIZE);
                }
            } else {
                run = 0;
            }
        }
        Err(FrameError::Exhausted)
    }

    fn locate(&self, address: u64) -> Result<(usize, u64), FrameError> {
        if !address.is_multiple_of(PAGE_SIZE) {
            return Err(FrameError::InvalidRange);
        }
        let frame = address / PAGE_SIZE;
        if frame >= self.managed.len() as u64 * 64 {
            return Err(FrameError::AddressLimit);
        }
        Ok((frame as usize / 64, 1 << (frame % 64)))
    }

    pub fn is_allocated(&self, address: u64) -> bool {
        self.locate(address)
            .is_ok_and(|(index, mask)| self.allocated[index] & mask != 0)
    }

    pub fn free_count(&self) -> usize {
        self.free
    }
    pub fn total_count(&self) -> usize {
        self.total
    }
}
