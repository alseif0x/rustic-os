// SPDX-License-Identifier: Apache-2.0
//! The v7 header copy: durable lineage, identity watermark, sequence, generation
//! and the three aggregate checksums of the generation it names.
//!
//! One copy per generation lives in sectors 8 and 9. Offsets inside a copy:
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0 | 8 | magic `RUSTFS3\0` |
//! | 8 | 1 | version 7 |
//! | 9 | 1 | layout marker |
//! | 10 | 1 | generation (0 or 1) |
//! | 11 | 1 | reserved zero |
//! | 12 | 16 | lineage |
//! | 28 | 8 | epoch |
//! | 36 | 8 | sequence |
//! | 44 | 4 | next identity watermark |
//! | 48 | 4 | node table checksum |
//! | 52 | 4 | map checksum |
//! | 56 | 4 | receipt block checksum |
//! | 60 | 4 | objects (256) |
//! | 64 | 4 | node bytes (128) |
//! | 68 | 4 | map bytes (16384) |
//! | 72 | 4 | payload sectors (131072) |
//! | 76 | 4 | retained records (8) |
//! | 80 | 4 | feature marker |
//! | 84 | 4 | record bytes (192) |
//! | 88 | 1 | extents per file (8) |
//! | 89 | 3 | reserved zero |
//! | 92 | 416 | reserved zero |
//! | 508 | 4 | CRC-32 over the copy with this field zeroed |

use super::{
    FEATURES, GENERATIONS, LAYOUT, MAGIC, MAP_BYTES, MAX_EXTENTS, NODE_BYTES, NODES, RECORD_BYTES,
    RETAINED, SECTOR_BYTES, VERSION,
};
use crate::Error;
use crate::checksum::crc;
use crate::extent::DATA_SECTORS;

/// Smallest watermark a volume may carry: identities 1..=4 name the reserved
/// roots, so the first allocated identity is 5.
pub const NEXT_MIN: u32 = 5;
/// A watermark of `u32::MAX` is exhausted: no identity remains to hand out. The
/// allocator must refuse instead of wrapping to an identity that already exists.
pub const NEXT_EXHAUSTED: u32 = u32::MAX;

/// The durable head of a v7 volume. `epoch` is the retry generation a client
/// binds to; `sequence` is the global transaction/version identity; `next` is the
/// monotonic allocator watermark, never a live-object count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header7 {
    pub lineage: [u8; 16],
    pub epoch: u64,
    pub sequence: u64,
    pub next: u32,
    /// The generation whose node table, map and receipt block the checksums
    /// describe.
    pub generation: u8,
    pub nodes_checksum: u32,
    pub map_checksum: u32,
    pub receipts_checksum: u32,
}

impl Header7 {
    pub const INITIAL_SEQUENCE: u64 = 1;
    pub const INITIAL_EPOCH: u64 = 1;
    pub const NEXT_MIN: u32 = NEXT_MIN;
    pub const NEXT_EXHAUSTED: u32 = NEXT_EXHAUSTED;

    /// A volume that has never been committed: generation 0, first sequence,
    /// no aggregate checksums yet and the first identity still to allocate.
    pub const fn initial(lineage: [u8; 16]) -> Self {
        Self {
            lineage,
            epoch: Self::INITIAL_EPOCH,
            sequence: Self::INITIAL_SEQUENCE,
            next: NEXT_MIN,
            generation: 0,
            nodes_checksum: 0,
            map_checksum: 0,
            receipts_checksum: 0,
        }
    }

    pub fn encode(&self) -> Result<[u8; SECTOR_BYTES as usize], Error> {
        self.validate()?;
        let mut b = [0; SECTOR_BYTES as usize];
        b[..8].copy_from_slice(&MAGIC);
        b[8] = VERSION;
        b[9] = LAYOUT;
        b[10] = self.generation;
        b[12..28].copy_from_slice(&self.lineage);
        b[28..36].copy_from_slice(&self.epoch.to_le_bytes());
        b[36..44].copy_from_slice(&self.sequence.to_le_bytes());
        b[44..48].copy_from_slice(&self.next.to_le_bytes());
        b[48..52].copy_from_slice(&self.nodes_checksum.to_le_bytes());
        b[52..56].copy_from_slice(&self.map_checksum.to_le_bytes());
        b[56..60].copy_from_slice(&self.receipts_checksum.to_le_bytes());
        b[60..64].copy_from_slice(&(NODES as u32).to_le_bytes());
        b[64..68].copy_from_slice(&(NODE_BYTES as u32).to_le_bytes());
        b[68..72].copy_from_slice(&(MAP_BYTES as u32).to_le_bytes());
        b[72..76].copy_from_slice(&(DATA_SECTORS as u32).to_le_bytes());
        b[76..80].copy_from_slice(&(RETAINED as u32).to_le_bytes());
        b[80..84].copy_from_slice(&FEATURES.to_le_bytes());
        b[84..88].copy_from_slice(&(RECORD_BYTES as u32).to_le_bytes());
        b[88] = MAX_EXTENTS as u8;
        let checksum = crc(&b);
        b[508..512].copy_from_slice(&checksum.to_le_bytes());
        Ok(b)
    }

    /// Refuses any copy this codec does not implement exactly: wrong format,
    /// wrong layout or feature marker, a reserved byte that is not zero, a
    /// geometry that is not the frozen one, a broken checksum, or a head whose
    /// own invariants fail.
    pub fn decode(b: &[u8; SECTOR_BYTES as usize]) -> Result<Self, Error> {
        let mut body = *b;
        let checksum = u32::from_le_bytes(body[508..512].try_into().unwrap());
        body[508..512].fill(0);
        if b[..8] != MAGIC
            || b[8] != VERSION
            || b[9] != LAYOUT
            || b[10] >= GENERATIONS
            || b[11] != 0
            || b[89..92] != [0; 3]
            || b[92..508].iter().any(|byte| *byte != 0)
            || u32::from_le_bytes(b[60..64].try_into().unwrap()) as usize != NODES
            || u32::from_le_bytes(b[64..68].try_into().unwrap()) as usize != NODE_BYTES
            || u32::from_le_bytes(b[68..72].try_into().unwrap()) as u64 != MAP_BYTES
            || u32::from_le_bytes(b[72..76].try_into().unwrap()) as u64 != DATA_SECTORS
            || u32::from_le_bytes(b[76..80].try_into().unwrap()) as usize != RETAINED
            || u32::from_le_bytes(b[80..84].try_into().unwrap()) != FEATURES
            || u32::from_le_bytes(b[84..88].try_into().unwrap()) as usize != RECORD_BYTES
            || b[88] as usize != MAX_EXTENTS
            || crc(&body) != checksum
        {
            return Err(Error::Corrupt);
        }
        let header = Self {
            lineage: b[12..28].try_into().unwrap(),
            epoch: u64::from_le_bytes(b[28..36].try_into().unwrap()),
            sequence: u64::from_le_bytes(b[36..44].try_into().unwrap()),
            next: u32::from_le_bytes(b[44..48].try_into().unwrap()),
            generation: b[10],
            nodes_checksum: u32::from_le_bytes(b[48..52].try_into().unwrap()),
            map_checksum: u32::from_le_bytes(b[52..56].try_into().unwrap()),
            receipts_checksum: u32::from_le_bytes(b[56..60].try_into().unwrap()),
        };
        header.validate()?;
        Ok(header)
    }

    /// The header's own invariants, which need no other structure: an unbroken
    /// lineage, a sequence that has started, an epoch that cannot be newer than
    /// the sequence it belongs to, and a watermark that has not run off the id
    /// space.
    pub fn validate(&self) -> Result<(), Error> {
        if self.lineage == [0; 16]
            || self.sequence < Self::INITIAL_SEQUENCE
            || self.epoch < Self::INITIAL_EPOCH
            || self.epoch > self.sequence
            || self.next < NEXT_MIN
            || self.generation >= GENERATIONS
        {
            return Err(Error::Corrupt);
        }
        Ok(())
    }

    /// The identity a later allocator may hand out, or [`Error::Exhausted`] once
    /// the watermark reaches [`NEXT_EXHAUSTED`]. This codec never allocates: it
    /// only fixes the boundary, so an allocator refuses overflow instead of
    /// wrapping onto an identity that already exists.
    pub fn next_id(&self) -> Result<u32, Error> {
        if self.next == NEXT_EXHAUSTED {
            Err(Error::Exhausted)
        } else {
            Ok(self.next)
        }
    }
}
