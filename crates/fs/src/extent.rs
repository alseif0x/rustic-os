// SPDX-License-Identifier: Apache-2.0
//! Payload extents and free-space accounting for the v6 layout (#51).
//!
//! The v5 volume addresses payload as `(slot, bank)` with one fixed kilobyte per
//! bank, so a file is one bank and 174 sectors bound the whole payload area. The
//! selected v6 shape keeps the copy-on-write control records and moves file bytes
//! into a dedicated region addressed by an extent list, with a free-space map
//! that is itself checksummed by the caller. This module is the pure half of that
//! decision: no disk, no kernel, no allocation.

use crate::Error;

/// Objects the v6 control records describe.
pub const OBJECTS_V6: usize = 256;
/// Largest single file in the v6 payload region.
pub const MAX_FILE_V6: usize = 256 * 1024;
/// Total payload region, the cap that makes exhaustion decidable.
pub const DATA_BYTES_V6: u64 = 64 * 1024 * 1024;
/// Allocation granularity: the volume's sector.
pub const SECTOR_BYTES: u64 = 512;
/// Sectors the whole payload region spans.
pub const DATA_SECTORS: u64 = DATA_BYTES_V6 / SECTOR_BYTES;
/// Words in the free-space bitmap for that region (one bit per sector).
pub const MAP_WORDS: usize = (DATA_SECTORS as usize).div_ceil(64);
/// Extents one file may reference. Bounding this bounds the control record.
pub const EXTENTS_PER_FILE: usize = 8;
/// Sectors one file may occupy, derived from its byte limit.
pub const FILE_SECTORS_MAX: u64 = (MAX_FILE_V6 as u64).div_ceil(SECTOR_BYTES);

/// One contiguous run of payload sectors. `start` is relative to the payload
/// region, not to the disk, so the region can move without rewriting extents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    pub start: u64,
    pub sectors: u64,
}

impl Extent {
    pub const fn new(start: u64, sectors: u64) -> Self {
        Self { start, sectors }
    }
    pub const fn end(&self) -> u64 {
        self.start + self.sectors
    }
}

/// A file's runs, in the order its bytes occupy them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extents {
    runs: [Option<Extent>; EXTENTS_PER_FILE],
    used: usize,
    sectors: u64,
}

impl Default for Extents {
    fn default() -> Self {
        Self::new()
    }
}

impl Extents {
    pub const fn new() -> Self {
        Self {
            runs: [None; EXTENTS_PER_FILE],
            used: 0,
            sectors: 0,
        }
    }
    /// Append a run. A file may reference at most `EXTENTS_PER_FILE` runs and may
    /// not exceed `FILE_SECTORS_MAX` sectors in total; both refusals are `Full`
    /// or `Size` respectively so exhaustion names its cause.
    pub fn push(&mut self, run: Extent) -> Result<(), Error> {
        if run.sectors == 0 {
            return Err(Error::Invalid);
        }
        if self.used == EXTENTS_PER_FILE {
            return Err(Error::Full);
        }
        let total = self.sectors.checked_add(run.sectors).ok_or(Error::Size)?;
        if total > FILE_SECTORS_MAX {
            return Err(Error::Size);
        }
        self.runs[self.used] = Some(run);
        self.used += 1;
        self.sectors = total;
        Ok(())
    }
    pub fn runs(&self) -> impl Iterator<Item = Extent> + '_ {
        self.runs[..self.used].iter().filter_map(|run| *run)
    }
    pub fn len(&self) -> usize {
        self.used
    }
    pub fn is_empty(&self) -> bool {
        self.used == 0
    }
    pub fn sectors(&self) -> u64 {
        self.sectors
    }
    pub fn bytes(&self) -> u64 {
        self.sectors * SECTOR_BYTES
    }
    /// Byte offset of `run`'s start inside the file, for range reads.
    pub fn offset_of(&self, index: usize) -> Option<u64> {
        if index >= self.used {
            return None;
        }
        let mut bytes = 0;
        for run in self.runs[..index].iter().flatten() {
            bytes += run.sectors * SECTOR_BYTES;
        }
        Some(bytes)
    }
}

/// One bit per payload sector. The owner owns the storage and must persist and
/// checksum it; this type only keeps the accounting honest.
pub struct FreeSpace<'a> {
    words: &'a mut [u64],
    free: u64,
}

impl<'a> FreeSpace<'a> {
    /// `words` must hold one bit per payload sector.
    pub fn new(words: &'a mut [u64]) -> Result<Self, Error> {
        if words.len() != MAP_WORDS {
            return Err(Error::Size);
        }
        let used_sectors = DATA_SECTORS;
        Ok(Self {
            words,
            free: used_sectors,
        })
    }
    /// Mark every sector used, for a region that is not yet formatted.
    pub fn fill(&mut self) {
        self.words.fill(u64::MAX);
        self.free = 0;
    }
    pub fn free_sectors(&self) -> u64 {
        self.free
    }
    pub fn free_bytes(&self) -> u64 {
        self.free * SECTOR_BYTES
    }
    fn get(&self, sector: u64) -> bool {
        self.words[sector as usize / 64] & (1 << (sector % 64)) != 0
    }
    fn set(&mut self, sector: u64, used: bool) {
        let word = &mut self.words[sector as usize / 64];
        let mask = 1 << (sector % 64);
        if used {
            *word |= mask;
        } else {
            *word &= !mask;
        }
    }
    /// First free run of `sectors`, or `Full`. The search is bounded by the
    /// region; no fragmentation policy is invented here.
    pub fn allocate(&mut self, sectors: u64) -> Result<Extent, Error> {
        if sectors == 0 || sectors > FILE_SECTORS_MAX {
            return Err(Error::Size);
        }
        if sectors > self.free {
            return Err(Error::Full);
        }
        let mut run = 0;
        for sector in 0..DATA_SECTORS {
            if self.get(sector) {
                run = 0;
                continue;
            }
            run += 1;
            if run == sectors {
                let start = sector + 1 - sectors;
                for taken in start..=sector {
                    self.set(taken, true);
                }
                self.free -= sectors;
                return Ok(Extent::new(start, sectors));
            }
        }
        Err(Error::Full)
    }
    /// Return a run. Releasing an unallocated sector is `Invalid`, so double
    /// release cannot silently corrupt the accounting.
    pub fn release(&mut self, run: Extent) -> Result<(), Error> {
        if run.sectors == 0 || run.end() > DATA_SECTORS {
            return Err(Error::Invalid);
        }
        for sector in run.start..run.end() {
            if !self.get(sector) {
                return Err(Error::Invalid);
            }
        }
        for sector in run.start..run.end() {
            self.set(sector, false);
        }
        self.free += run.sectors;
        Ok(())
    }
}
