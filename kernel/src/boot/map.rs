// SPDX-License-Identifier: Apache-2.0
use super::MemoryRegion;

/// Summary only; does not grant ownership or allocate any pages.
#[derive(Debug, PartialEq, Eq)]
pub struct MapSummary {
    pub entries: usize,
    pub usable_bytes: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MapError {
    InvalidRange,
    UnorderedOrOverlapping,
    TooManyEntries,
    NoUsableMemory,
}

/// Validate a sorted firmware map with bounded iteration and checked arithmetic.
/// Unknown region types must be passed as non-usable by the protocol adapter.
pub fn validate_map(
    entries: impl IntoIterator<Item = (u64, u64, bool)>,
) -> Result<MapSummary, MapError> {
    let mut summary = MapSummary {
        entries: 0,
        usable_bytes: 0,
    };
    let mut previous_end = 0;
    for (base, length, usable) in entries {
        if summary.entries == 4096 {
            return Err(MapError::TooManyEntries);
        }
        let region = MemoryRegion::new(base, length).map_err(|_| MapError::InvalidRange)?;
        if base < previous_end {
            return Err(MapError::UnorderedOrOverlapping);
        }
        previous_end = region.end();
        summary.entries += 1;
        if usable {
            summary.usable_bytes = summary
                .usable_bytes
                .checked_add(length)
                .ok_or(MapError::InvalidRange)?;
        }
    }
    if summary.usable_bytes == 0 {
        return Err(MapError::NoUsableMemory);
    }
    Ok(summary)
}
