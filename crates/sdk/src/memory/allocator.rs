// SPDX-License-Identifier: Apache-2.0
//! Bounded first-fit block allocator over one contiguous byte region.
//!
//! The free list lives inside the region: each block carries its own total size,
//! the total size of the block before it and its state, so blocks tile the
//! region and both neighbours are reachable without a side table. Allocation
//! walks that tiling and takes the first run that fits the requested size and
//! alignment; releasing coalesces with the previous and the next block.
//!
//! Header layout, three machine words at the start of every block:
//!
//! | Word | Meaning |
//! | --- | --- |
//! | 0 | Total bytes of this block, header included |
//! | 1 | Total bytes of the preceding block, `0` for the first block |
//! | 2 | Generation and state, `generation * 2 + state` |
//!
//! The third word is the state marker the region leaves to the allocator. Its
//! low bit is free or used; the remaining bits hold the generation of the
//! allocation that occupies the block, taken from a counter that never repeats
//! a value in one region. A free block carries generation `0`. Because
//! a [`Block`] handle records the generation it was issued with, a stale copy
//! is refused even after its address has been handed out again.
//!
//! This module is pure policy and portable: every raw access goes through the
//! bounds-checked [`super::region::Region`], so host tests drive exactly the
//! code the guest heap runs. It is not a `GlobalAlloc` and never grows itself;
//! growing and shrinking the region is the owner's decision.
use super::region::{ALIGN, HEADER, Header, Region};
use core::ptr::NonNull;

const FREE: usize = 0;
const USED: usize = 1;
/// Low bit of the third header word; the generation occupies the rest.
const STATE: usize = 1;

/// Free or used marker of a block.
fn state(header: &Header) -> usize {
    header.state & STATE
}

/// Generation of the allocation that occupies a block, `0` while it is free.
fn generation(header: &Header) -> usize {
    header.state >> 1
}

/// Packs a generation and a state marker into the third header word.
fn tag(generation: usize, state: usize) -> usize {
    (generation << 1) | state
}

/// Distinct refusals; nothing is rounded away or silently ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocError {
    /// Zero size, an alignment that is not a power of two, or above `MAX_ALIGN`.
    Request,
    /// No free byte remains in the region.
    Empty,
    /// Free bytes remain, but no single run satisfies size plus alignment.
    Exhausted,
    /// The address is outside the region, is not the start of a live block, or
    /// belongs to a released allocation whose address has been reused since.
    Unowned,
    /// The address belongs to a block that is already free.
    DoubleFree,
    /// The byte range cannot carry the bookkeeping this operation needs.
    Region,
}

/// A live allocation: where it starts, how many bytes the caller asked for and
/// which generation of that address the handle refers to.
///
/// The handle carries no authority of its own: it is validated against the
/// owning allocator on every use, and the generation makes that check exact, so
/// a foreign copy, or a copy of a handle whose address has since been handed
/// out again, is refused as [`AllocError::Unowned`] instead of trusted. Copying
/// a handle is therefore harmless; releasing the allocation invalidates every
/// copy at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    ptr: NonNull<u8>,
    size: usize,
    generation: usize,
}

impl Block {
    pub fn address(&self) -> usize {
        self.ptr.addr().get()
    }

    pub fn size(&self) -> usize {
        self.size
    }
}

/// One consistent snapshot of the region's accounting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Bytes the region spans, bookkeeping included.
    pub capacity: usize,
    /// Bytes committed to live allocations, their headers and their padding.
    pub used: usize,
    /// `capacity - used`, spread over one or more free runs.
    pub free: usize,
    /// Largest single free run; a request above it cannot be satisfied.
    pub largest_free: usize,
    /// Highest `used` ever observed in this region.
    pub peak_used: usize,
    /// Number of live allocations.
    pub blocks: usize,
}

pub struct Allocator<'a> {
    region: Region<'a>,
    used: usize,
    live: usize,
    peak: usize,
    /// Generation the next allocation receives; never `0` and never reissued.
    generation: usize,
}

impl<'a> Allocator<'a> {
    /// Largest alignment a single allocation may request.
    pub const MAX_ALIGN: usize = 4096;
    /// Bookkeeping bytes charged to every block in addition to its payload.
    pub const OVERHEAD: usize = HEADER;

    /// Takes exclusive ownership of a caller-provided buffer.
    pub fn new(bytes: &'a mut [u8]) -> Result<Self, AllocError> {
        Self::initialise(Region::new(bytes).ok_or(AllocError::Region)?)
    }

    /// # Safety
    /// `base` must address `len` writable, initialized bytes that the caller
    /// owns exclusively and keeps mapped for at least `'a`, and that no other
    /// allocator manages. The guest heap satisfies this with mapped pages.
    pub unsafe fn from_raw(base: NonNull<u8>, len: usize) -> Result<Self, AllocError> {
        // SAFETY: forwarded unchanged from this function's own contract.
        let region = unsafe { Region::from_raw(base, len) }.ok_or(AllocError::Region)?;
        Self::initialise(region)
    }

    fn initialise(mut region: Region<'a>) -> Result<Self, AllocError> {
        let len = region.len();
        if len < HEADER
            || !region.set_header(
                0,
                Header {
                    size: len,
                    prev: 0,
                    state: tag(0, FREE),
                },
            )
        {
            return Err(AllocError::Region);
        }
        Ok(Self {
            region,
            used: 0,
            live: 0,
            peak: 0,
            generation: 1,
        })
    }

    /// First address of the region; every block lies inside it.
    pub fn address(&self) -> usize {
        self.region.address()
    }

    /// Bytes the region currently spans, bookkeeping included.
    pub fn capacity(&self) -> usize {
        self.region.len()
    }

    /// Bytes committed to live allocations, their headers and their padding.
    pub fn used_bytes(&self) -> usize {
        self.used
    }

    /// Bytes not committed to any live allocation. `used + free == capacity`.
    pub fn free_bytes(&self) -> usize {
        self.region.len() - self.used
    }

    /// Highest `used_bytes` ever observed, for bounded reporting.
    pub fn peak_used_bytes(&self) -> usize {
        self.peak
    }

    /// Number of live allocations.
    pub fn blocks(&self) -> usize {
        self.live
    }

    /// Largest single free run, bookkeeping included; `0` when nothing is free.
    pub fn largest_free(&self) -> usize {
        let mut offset = 0;
        let mut largest = 0;
        while let Some((next, header)) = self.step(offset) {
            if state(&header) == FREE && header.size > largest {
                largest = header.size;
            }
            offset = next;
        }
        largest
    }

    pub fn stats(&self) -> Stats {
        Stats {
            capacity: self.capacity(),
            used: self.used,
            free: self.free_bytes(),
            largest_free: self.largest_free(),
            peak_used: self.peak,
            blocks: self.live,
        }
    }

    pub fn alloc(&mut self, size: usize, align: usize) -> Result<Block, AllocError> {
        if size == 0 || !align.is_power_of_two() || align > Self::MAX_ALIGN {
            return Err(AllocError::Request);
        }
        let need = size
            .checked_next_multiple_of(ALIGN)
            .ok_or(AllocError::Request)?;
        let base = self.region.address();
        let mut offset = 0;
        let found = loop {
            let Some((next, header)) = self.step(offset) else {
                break None;
            };
            if state(&header) == FREE
                && let Some(gap) = leading_gap(base + offset, align)
                && gap + HEADER + need <= header.size
            {
                break Some((offset, header, gap));
            }
            offset = next;
        };
        let (offset, header, gap) = found.ok_or(if self.free_bytes() == 0 {
            AllocError::Empty
        } else {
            AllocError::Exhausted
        })?;
        let end = offset + header.size;
        let mut start = offset;
        let mut block = header.size;
        let mut prev = header.prev;
        if gap > 0 {
            // Alignment padding becomes an ordinary free block of its own, so
            // every used block still starts exactly HEADER before its payload.
            self.write(
                offset,
                Header {
                    size: gap,
                    prev,
                    state: tag(0, FREE),
                },
            )?;
            start = offset + gap;
            block = header.size - gap;
            prev = gap;
        }
        let want = HEADER + need;
        let mut last = block;
        if block >= want + HEADER {
            self.write(
                start + want,
                Header {
                    size: block - want,
                    prev: want,
                    state: tag(0, FREE),
                },
            )?;
            last = block - want;
            block = want;
        }
        let generation = self.generation;
        self.write(
            start,
            Header {
                size: block,
                prev,
                state: tag(generation, USED),
            },
        )?;
        self.relink(end, last)?;
        self.used += block;
        self.live += 1;
        // Wrapping needs 2^63 allocations in one region before a generation is
        // reissued; no bounded guest reaches that, and the mask keeps the
        // packed word from ever overflowing into a different block's state.
        self.generation = generation.wrapping_add(1) & (usize::MAX >> 1);
        if self.generation == 0 {
            self.generation = 1;
        }
        if self.used > self.peak {
            self.peak = self.used;
        }
        let ptr = self
            .region
            .pointer(start + HEADER)
            .ok_or(AllocError::Region)?;
        Ok(Block {
            ptr,
            size,
            generation,
        })
    }

    pub fn free(&mut self, block: Block) -> Result<(), AllocError> {
        let offset = self.start_of(&block)?;
        let mut header = self.read(offset)?;
        // Clearing the whole word also drops the generation, so every copy of
        // the handle stops matching this block from here on.
        header.state = tag(0, FREE);
        self.write(offset, header)?;
        self.used -= header.size;
        self.live -= 1;
        self.coalesce(offset)
    }

    pub fn bytes(&self, block: &Block) -> Result<&[u8], AllocError> {
        let offset = self.start_of(block)?;
        self.region
            .bytes(offset + HEADER, block.size)
            .ok_or(AllocError::Region)
    }

    pub fn bytes_mut(&mut self, block: &Block) -> Result<&mut [u8], AllocError> {
        let offset = self.start_of(block)?;
        self.region
            .bytes_mut(offset + HEADER, block.size)
            .ok_or(AllocError::Region)
    }

    /// Appends `extra` owned bytes directly after the region.
    ///
    /// When the region already ends in a free block that block is enlarged in
    /// place and not one byte of the new range is written, so a caller can
    /// still observe those pages exactly as the kernel granted them.
    ///
    /// # Safety
    /// The `extra` bytes immediately after `address() + capacity()` must be
    /// owned exclusively by this allocator's owner, writable, initialized and
    /// kept mapped for as long as the allocator exists.
    pub unsafe fn extend(&mut self, extra: usize) -> Result<(), AllocError> {
        if extra < HEADER || !extra.is_multiple_of(ALIGN) {
            return Err(AllocError::Region);
        }
        let len = self.region.len();
        let grown = len.checked_add(extra).ok_or(AllocError::Region)?;
        // Growing the region cannot be undone, so the whole bookkeeping change
        // is decided and validated against the grown length first; only then is
        // a byte of the new range accounted for, and the write cannot refuse.
        let (offset, header) = match self.last() {
            Some((offset, mut header)) if state(&header) == FREE => {
                header.size = header.size.checked_add(extra).ok_or(AllocError::Region)?;
                (offset, header)
            }
            Some((_, header)) => (
                len,
                Header {
                    size: extra,
                    prev: header.size,
                    state: tag(0, FREE),
                },
            ),
            None => (
                len,
                Header {
                    size: extra,
                    prev: 0,
                    state: tag(0, FREE),
                },
            ),
        };
        if !offset.is_multiple_of(ALIGN) || offset.checked_add(HEADER).is_none_or(|end| end > grown)
        {
            return Err(AllocError::Region);
        }
        // SAFETY: forwarded unchanged from this function's own contract.
        unsafe { self.region.grow(extra) };
        self.write(offset, header)
    }

    /// Largest multiple of `granularity` the region may shrink to without
    /// touching a live block; `capacity()` when nothing can be released.
    pub fn truncation_target(&self, granularity: usize) -> usize {
        let len = self.region.len();
        if !granularity.is_power_of_two() {
            return len;
        }
        let Some((offset, header)) = self.last() else {
            return len;
        };
        if state(&header) != FREE {
            return len;
        }
        // Either the trailing free block disappears entirely, or what is left
        // of it must still be able to carry a header of its own.
        let target = if offset.is_multiple_of(granularity) {
            offset
        } else {
            match (offset + HEADER).checked_next_multiple_of(granularity) {
                Some(target) => target,
                None => return len,
            }
        };
        if target >= len { len } else { target }
    }

    /// Gives up the bytes after `len`. Only a trailing free run may be dropped,
    /// and `len` must keep the region's alignment invariant; a target from
    /// [`Allocator::truncation_target`] satisfies both and is always accepted.
    pub fn truncate(&mut self, len: usize) -> Result<(), AllocError> {
        if !len.is_multiple_of(ALIGN) {
            return Err(AllocError::Region);
        }
        let current = self.region.len();
        if len == current {
            return Ok(());
        }
        let (offset, mut header) = self.last().ok_or(AllocError::Region)?;
        if len > current || state(&header) != FREE || len < offset {
            return Err(AllocError::Region);
        }
        let remainder = len - offset;
        if remainder != 0 {
            if remainder < HEADER {
                return Err(AllocError::Region);
            }
            header.size = remainder;
            self.write(offset, header)?;
        }
        if !self.region.shrink(len) {
            return Err(AllocError::Region);
        }
        Ok(())
    }

    /// Offset of the block that owns `block`, rejecting every other address.
    fn start_of(&self, block: &Block) -> Result<usize, AllocError> {
        let base = self.region.address();
        let address = block.address();
        if address < base || address - base >= self.region.len() {
            return Err(AllocError::Unowned);
        }
        let target = address - base;
        let mut offset = 0;
        while let Some((next, header)) = self.step(offset) {
            if target < next {
                if state(&header) == FREE {
                    return Err(AllocError::DoubleFree);
                }
                // The generation settles which allocation the handle refers to:
                // a copy left over from an earlier one at this exact address
                // and size no longer matches the block living there now.
                if target == offset + HEADER
                    && block.size <= header.size - HEADER
                    && block.generation == generation(&header)
                {
                    return Ok(offset);
                }
                return Err(AllocError::Unowned);
            }
            offset = next;
        }
        Err(AllocError::Unowned)
    }

    /// Merges a just-freed block with a free next and a free previous neighbour.
    fn coalesce(&mut self, offset: usize) -> Result<(), AllocError> {
        let mut header = self.read(offset)?;
        let next = offset + header.size;
        if next + HEADER <= self.region.len() {
            let following = self.read(next)?;
            if state(&following) == FREE {
                header.size += following.size;
                self.write(offset, header)?;
            }
        }
        let mut start = offset;
        if header.prev != 0 {
            let previous = offset - header.prev;
            let mut merged = self.read(previous)?;
            if state(&merged) == FREE {
                merged.size += header.size;
                self.write(previous, merged)?;
                start = previous;
                header = merged;
            }
        }
        self.relink(start + header.size, header.size)
    }

    /// Records `size` as the predecessor of the block that starts at `offset`.
    fn relink(&mut self, offset: usize, size: usize) -> Result<(), AllocError> {
        if offset + HEADER > self.region.len() {
            return Ok(());
        }
        let mut header = self.read(offset)?;
        header.prev = size;
        self.write(offset, header)
    }

    /// Header at `offset` and the offset of the block after it, or `None` at
    /// the end of the region. A malformed size also stops the walk.
    fn step(&self, offset: usize) -> Option<(usize, Header)> {
        if offset + HEADER > self.region.len() {
            return None;
        }
        let header = self.region.header(offset)?;
        if header.size < HEADER || offset + header.size > self.region.len() {
            return None;
        }
        Some((offset + header.size, header))
    }

    fn last(&self) -> Option<(usize, Header)> {
        let mut offset = 0;
        let mut found = None;
        while let Some((next, header)) = self.step(offset) {
            found = Some((offset, header));
            offset = next;
        }
        found
    }

    fn read(&self, offset: usize) -> Result<Header, AllocError> {
        self.region.header(offset).ok_or(AllocError::Region)
    }

    fn write(&mut self, offset: usize, header: Header) -> Result<(), AllocError> {
        if self.region.set_header(offset, header) {
            Ok(())
        } else {
            Err(AllocError::Region)
        }
    }
}

/// Bytes that must precede a block at `block` so its payload reaches `align`.
///
/// A non-zero gap becomes a standalone free block, so it is pushed forward
/// until it can carry a header of its own.
fn leading_gap(block: usize, align: usize) -> Option<usize> {
    let payload = block.checked_add(HEADER)?.checked_next_multiple_of(align)?;
    let mut gap = payload - HEADER - block;
    while gap != 0 && gap < HEADER {
        gap = gap.checked_add(align)?;
    }
    Some(gap)
}
