// SPDX-License-Identifier: Apache-2.0

/// A nonempty half-open physical address range supplied during boot.
///
/// This validates arithmetic only. It does not establish that the range is
/// usable RAM, mapped, owned by the kernel, or nonoverlapping with other ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRegion {
    start: u64,
    end: u64,
}

/// Reasons an address range cannot be represented safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionError {
    Empty,
    AddressOverflow,
}

impl MemoryRegion {
    /// Validates a base address and byte length without wrapping the end address.
    pub const fn new(start: u64, length: u64) -> Result<Self, RegionError> {
        if length == 0 {
            return Err(RegionError::Empty);
        }
        match start.checked_add(length) {
            Some(end) => Ok(Self { start, end }),
            None => Err(RegionError::AddressOverflow),
        }
    }

    pub const fn start(self) -> u64 {
        self.start
    }

    pub const fn end(self) -> u64 {
        self.end
    }

    /// Tests containment of a single address; the end is exclusive.
    pub const fn contains(self, address: u64) -> bool {
        self.start <= address && address < self.end
    }

    /// Adjacent regions do not overlap.
    pub const fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}
