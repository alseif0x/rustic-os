// SPDX-License-Identifier: Apache-2.0
//! Bounded static ELF64 subset. Decode bytes without aligned casts or allocation.
use crate::memory::PAGE_SIZE;

pub const MAX_SEGMENTS: usize = 8;
pub const MAX_PAGES: u64 = 256;
pub const STACK_TOP: u64 = 0x8000_0000;
pub const STACK_PAGES: u64 = 16;
pub const STACK_GUARD: u64 = STACK_TOP - (STACK_PAGES + 1) * PAGE_SIZE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Header,
    Bounds,
    Unsupported,
    Segment,
    Overlap,
    Budget,
    Entry,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Segment {
    pub address: u64,
    pub memory_size: u64,
    pub offset: usize,
    pub file_size: usize,
    pub writable: bool,
    pub executable: bool,
}

impl Segment {
    pub fn start_page(self) -> u64 {
        self.address & !(PAGE_SIZE - 1)
    }
    pub fn end_page(self) -> u64 {
        (self.address + self.memory_size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
    }
}

#[derive(Debug)]
pub struct Image<'a> {
    bytes: &'a [u8],
    entry: u64,
    segments: [Segment; MAX_SEGMENTS],
    count: usize,
}

fn number(bytes: &[u8], at: usize, size: usize) -> Result<u64, Error> {
    let end = at.checked_add(size).ok_or(Error::Bounds)?;
    let field = bytes.get(at..end).ok_or(Error::Bounds)?;
    Ok(field
        .iter()
        .enumerate()
        .fold(0, |n, (i, b)| n | (u64::from(*b) << (i * 8))))
}

impl<'a> Image<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < 64
            || bytes.len() > 1024 * 1024
            || bytes.get(..9) != Some(b"\x7fELF\x02\x01\x01\x00\x00")
            || bytes[9..16].iter().any(|b| *b != 0)
            || number(bytes, 16, 2)? != 2
            || number(bytes, 18, 2)? != 62
            || number(bytes, 20, 4)? != 1
            || number(bytes, 48, 4)? != 0
            || number(bytes, 52, 2)? != 64
            || number(bytes, 54, 2)? != 56
        {
            return Err(Error::Header);
        }
        let offset = usize::try_from(number(bytes, 32, 8)?).map_err(|_| Error::Bounds)?;
        let count = number(bytes, 56, 2)? as usize;
        if !(1..=16).contains(&count) {
            return Err(Error::Budget);
        }
        if offset < 64
            || offset
                .checked_add(count * 56)
                .is_none_or(|n| n > bytes.len())
        {
            return Err(Error::Bounds);
        }
        let mut image = Self {
            bytes,
            entry: number(bytes, 24, 8)?,
            segments: [Segment::default(); MAX_SEGMENTS],
            count: 0,
        };
        let mut pages = STACK_PAGES;
        let mut previous_end = PAGE_SIZE;
        for index in 0..count {
            let at = offset + index * 56;
            match number(bytes, at, 4)? {
                0 => continue,
                1 => {}
                // R0 has no dynamic linker, TLS, interpreter or executable stack.
                _ => return Err(Error::Unsupported),
            }
            let flags = number(bytes, at + 4, 4)?;
            let file_offset = number(bytes, at + 8, 8)?;
            let address = number(bytes, at + 16, 8)?;
            let file_size = number(bytes, at + 32, 8)?;
            let memory_size = number(bytes, at + 40, 8)?;
            let alignment = number(bytes, at + 48, 8)?;
            let end = address.checked_add(memory_size).ok_or(Error::Bounds)?;
            if !matches!(flags, 4..=6)
                || memory_size == 0
                || file_size > memory_size
                || address < PAGE_SIZE
                || end > STACK_GUARD
                || file_offset % PAGE_SIZE != address % PAGE_SIZE
                || (alignment > 1
                    && (!alignment.is_power_of_two()
                        || address % alignment != file_offset % alignment))
            {
                return Err(Error::Segment);
            }
            if file_offset
                .checked_add(file_size)
                .is_none_or(|end| end > bytes.len() as u64)
            {
                return Err(Error::Bounds);
            }
            let segment = Segment {
                address,
                memory_size,
                offset: file_offset as usize,
                file_size: file_size as usize,
                writable: flags & 2 != 0,
                executable: flags & 1 != 0,
            };
            if segment.start_page() < previous_end {
                return Err(Error::Overlap);
            }
            previous_end = segment.end_page();
            pages += (segment.end_page() - segment.start_page()) / PAGE_SIZE;
            if pages > MAX_PAGES || image.count == MAX_SEGMENTS {
                return Err(Error::Budget);
            }
            image.segments[image.count] = segment;
            image.count += 1;
        }
        if !image.segments().iter().any(|s| {
            s.executable && (s.address..s.address + s.file_size as u64).contains(&image.entry)
        }) {
            return Err(Error::Entry);
        }
        Ok(image)
    }

    pub fn entry(&self) -> u64 {
        self.entry
    }
    pub fn segments(&self) -> &[Segment] {
        &self.segments[..self.count]
    }
    pub fn data(&self, segment: &Segment) -> &[u8] {
        &self.bytes[segment.offset..segment.offset + segment.file_size]
    }
}
