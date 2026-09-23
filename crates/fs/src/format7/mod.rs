// SPDX-License-Identifier: Apache-2.0
//! v7 immutable-content workspace contract: codecs and structural validation.
//!
//! This module fixes the on-disk shapes, checks each record and a decoded
//! generation's namespace and allocation ownership. It stays pure: the separate
//! `Volume7` owner provisions and verifies mounts, and currently publishes a
//! direct tracked replacement for an existing file. Staged admission,
//! cancellation, retention maintenance, migration and service support remain
//! later stages. The v5 and v6 layouts stay frozen, and these codecs do not claim
//! service support.
//!
//! Sectors are 512 bytes, numbered from the volume start:
//!
//! | sector | contents |
//! |---|---|
//! | 8 | header copy for generation 0 |
//! | 9 | header copy for generation 1 |
//! | 10..110 | generation 0: 64 node sectors, 32 map sectors, 4 receipt sectors |
//! | 110..210 | generation 1: the same 100-sector shape |
//! | 210.. | payload: 64 MiB of 512-byte sectors |
//!
//! A header is keyed by generation: the copy in sector `8 + generation` names the
//! checksums of that generation's node table, free-space map and receipt block.
//! The direct-commit publisher writes the inactive generation before its header.
//! This codec module does not perform I/O or establish crash safety. The
//! `Volume7` owner validates the candidate and enforces payload/metadata and
//! header flush ordering; safe outcome reclamation remains future work.
//!
//! Validation is split on purpose. [`Node7`] and [`Record7`] check what a single
//! record can prove about itself. [`Header7::validate`] and [`Record7::validate`]
//! add contextual bounds, and [`validate_generation`] checks namespace and
//! allocation ownership across decoded structures. It is read-only: it does not
//! read disk sectors or payload bytes and does not establish payload integrity.

use crate::Error;
use crate::checksum::crc;
use crate::extent::{
    DATA_BYTES_V6, DATA_SECTORS, EXTENTS_PER_FILE, Extent, MAX_FILE_V6, OBJECTS_V6,
};
use crate::receipt6::RETAINED_V6;

mod header;
mod node;
mod record;
mod validation;

pub use header::{Header7, NEXT_EXHAUSTED, NEXT_MIN};
pub use node::{NAME_BYTES, Node7};
pub use record::{Record7, RecordState, receipt_slots};
pub use validation::validate_generation;

/// Distinct from v5's `RUSTFS1` and v6's `RUSTFS2`, so no older volume is
/// misread as v7.
pub const MAGIC: [u8; 8] = *b"RUSTFS3\0";
pub const VERSION: u8 = 7;
pub const LAYOUT: u8 = 1;
pub const SECTOR_BYTES: u64 = 512;

/// One control record per object. The node table is bounded, so its size is a
/// constant and the header can name the table's checksum.
pub const NODE_BYTES: usize = 128;
/// Live objects the control records describe. Reused from the selected #51
/// extent budget; it is a capacity, not the identity watermark.
pub const NODES: usize = OBJECTS_V6;
pub const NODES_SECTORS: u64 = (NODES * NODE_BYTES) as u64 / SECTOR_BYTES;

/// One bit per payload sector, owned and persisted by a later stage.
pub const MAP_WORDS: usize = crate::extent::MAP_WORDS;
pub const MAP_BYTES: u64 = MAP_WORDS as u64 * 8;
pub const MAP_SECTORS: u64 = MAP_BYTES / SECTOR_BYTES;

/// Retained durable records. Reused from the selected #51 budget: a full table
/// reports `Full` instead of dropping an unresolved outcome.
pub const RETAINED: usize = RETAINED_V6;
/// On-disk size of one durable record, fixed by its field map.
pub const RECORD_BYTES: usize = 192;
/// Bytes of retained records one generation's receipt block carries. The block is
/// padded to four sectors; the padding is reserved and must be zero.
pub const RECORDS_BYTES: usize = RETAINED * RECORD_BYTES;
pub const RECEIPT_BLOCK_BYTES: usize = 4 * SECTOR_BYTES as usize;
pub const RECEIPT_RESERVED_BYTES: usize = RECEIPT_BLOCK_BYTES - RECORDS_BYTES;
pub const RECEIPTS_SECTORS: u64 = RECEIPT_BLOCK_BYTES as u64 / SECTOR_BYTES;

/// Two generations give a publication two places to write, one of which the
/// header can name after the other is durable. The writer owns that ordering and
/// the mount validation that trusts it; this is geometry only.
pub const GENERATIONS: u8 = 2;
pub const GENERATION_SECTORS: u64 = NODES_SECTORS + MAP_SECTORS + RECEIPTS_SECTORS;
/// First header sector; the copy for a generation is `HEADER_SECTOR + generation`.
pub const HEADER_SECTOR: u64 = 8;
/// First sector of generation 0's node table.
pub const FIRST_GENERATION_SECTOR: u64 = HEADER_SECTOR + GENERATIONS as u64;
/// First payload sector; extents are relative to it.
pub const PAYLOAD_SECTOR: u64 = FIRST_GENERATION_SECTOR + GENERATIONS as u64 * GENERATION_SECTORS;
pub const PAYLOAD_BYTES: u64 = DATA_BYTES_V6;
pub const VOLUME_SECTORS: u64 = PAYLOAD_SECTOR + DATA_SECTORS;

/// Largest file the payload addresses. Reused from the selected #51 budget.
pub const MAX_FILE_BYTES: u32 = MAX_FILE_V6 as u32;
/// Runs one file, node or retained record may reference.
pub const MAX_EXTENTS: usize = EXTENTS_PER_FILE;

/// Fixed feature marker. A reader refuses a feature it cannot honour, so the
/// header carries the exact set this codec implements and nothing else.
pub const FEATURE_EXTENT_PAYLOAD: u32 = 1;
pub const FEATURE_EXTENT_SNAPSHOT: u32 = 1 << 1;
pub const FEATURE_SCOPED_RECORDS: u32 = 1 << 2;
pub const FEATURES: u32 = FEATURE_EXTENT_PAYLOAD | FEATURE_EXTENT_SNAPSHOT | FEATURE_SCOPED_RECORDS;

/// First sector of a generation's header copy. Callers pass 0 or 1; a larger
/// value wraps into the two copies.
pub fn header_sector(generation: u8) -> u64 {
    HEADER_SECTOR + u64::from(generation % GENERATIONS)
}
/// First sector of a generation's node table.
pub fn nodes_sector(generation: u8) -> u64 {
    FIRST_GENERATION_SECTOR + u64::from(generation % GENERATIONS) * GENERATION_SECTORS
}
/// First sector of a generation's free-space map.
pub fn map_sector(generation: u8) -> u64 {
    nodes_sector(generation) + NODES_SECTORS
}
/// First sector of a generation's receipt block.
pub fn receipts_sector(generation: u8) -> u64 {
    map_sector(generation) + MAP_SECTORS
}

/// The CRC-32/IEEE aggregate a header names for a generation region. A later
/// stage owns reading the bytes and persists the result; this only fixes which
/// function the header's fields mean.
pub fn aggregate(bytes: &[u8]) -> u32 {
    crc(bytes)
}

/// Payload geometry shared by nodes and retained records: the first `used` runs
/// of `runs` must be non-zero, in bounds and non-overlapping, the runs after them
/// must be zero, and the used runs must total exactly the sectors `length` needs.
/// Claiming fewer sectors than the length, or more, is corrupt: a retained record
/// must describe the exact bytes it can replay.
fn check_payload(runs: &[Extent], used: u8, length: u32) -> Result<(), Error> {
    let used = usize::from(used);
    if used > runs.len() || u64::from(length) > u64::from(MAX_FILE_BYTES) {
        return Err(Error::Corrupt);
    }
    for run in &runs[..used] {
        if run.sectors == 0
            || run
                .start
                .checked_add(run.sectors)
                .is_none_or(|end| end > DATA_SECTORS)
        {
            return Err(Error::Corrupt);
        }
    }
    for (index, run) in runs[..used].iter().enumerate() {
        if runs[index + 1..used]
            .iter()
            .any(|other| run.start < other.end() && other.start < run.end())
        {
            return Err(Error::Corrupt);
        }
    }
    for run in &runs[used..] {
        if run.start != 0 || run.sectors != 0 {
            return Err(Error::Corrupt);
        }
    }
    let sectors: u64 = runs[..used].iter().map(|run| run.sectors).sum();
    if sectors != u64::from(length).div_ceil(SECTOR_BYTES) {
        return Err(Error::Corrupt);
    }
    Ok(())
}
