// SPDX-License-Identifier: Apache-2.0
//! v6 volume structures: widened control records and an extent-addressed payload.
//!
//! The v5 layout stores one 64-byte record per object and addresses payload as
//! `(slot, bank)`, which makes a file exactly one kilobyte. This module defines
//! the replacement records that the selected #51 shape needs: a 128-byte node
//! with a 32-bit length and up to [`EXTENTS_PER_FILE`] runs, a header that names
//! the region geometry and the checksums, and a free-space map block. It is pure
//! encoding and decoding; mounting and upgrading a volume are separate stages.

use crate::checksum::crc;
use crate::extent::{DATA_SECTORS, EXTENTS_PER_FILE, Extent};
use crate::{Error, Kind, OBJECTS_V6};

/// Distinct from v5's `RUSTFS1`, so a v5 volume is never misread as v6.
pub const MAGIC: [u8; 8] = *b"RUSTFS2\0";
pub const VERSION: u8 = 6;
pub const SECTOR_BYTES: u64 = 512;
/// One control record: bounded so the node table has a known size.
pub const NODE_BYTES: usize = 128;
/// Header sector, the node table and the map, in sectors from the volume start.
pub const HEADER_SECTOR: u64 = 8;
pub const NODES_SECTOR: u64 = HEADER_SECTOR + 1;
pub const NODES_SECTORS: u64 = (OBJECTS_V6 * NODE_BYTES) as u64 / SECTOR_BYTES;
pub const MAP_SECTOR: u64 = NODES_SECTOR + NODES_SECTORS;
pub const MAP_BYTES: u64 = super::extent::MAP_WORDS as u64 * 8;
pub const MAP_SECTORS: u64 = MAP_BYTES / SECTOR_BYTES;
/// First sector of the payload region; extents are relative to it.
pub const PAYLOAD_SECTOR: u64 = MAP_SECTOR + MAP_SECTORS;
/// Total sectors the v6 volume occupies, payload included.
pub const VOLUME_SECTORS: u64 = PAYLOAD_SECTOR + DATA_SECTORS;

/// A v6 control record. `length` is 32 bits, so the selected 256 KiB file limit
/// is structural rather than a policy check against a 16-bit field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node6 {
    pub id: u32,
    pub parent: u32,
    pub version: u64,
    pub length: u32,
    pub kind: Kind,
    pub space: u8,
    pub extents: [Extent; EXTENTS_PER_FILE],
    pub extents_used: u8,
    pub name: [u8; 32],
    pub name_length: u8,
}

impl Node6 {
    pub const EMPTY: Self = Self {
        id: 0,
        parent: 0,
        version: 0,
        length: 0,
        kind: Kind::Empty,
        space: 0,
        extents: [Extent::new(0, 0); EXTENTS_PER_FILE],
        extents_used: 0,
        name: [0; 32],
        name_length: 0,
    };

    pub fn encode(&self) -> [u8; NODE_BYTES] {
        let mut b = [0; NODE_BYTES];
        b[0..4].copy_from_slice(&self.id.to_le_bytes());
        b[4..8].copy_from_slice(&self.parent.to_le_bytes());
        b[8..16].copy_from_slice(&self.version.to_le_bytes());
        b[16..20].copy_from_slice(&self.length.to_le_bytes());
        b[20] = self.kind as u8;
        b[21] = self.space;
        b[22] = self.extents_used;
        for (index, run) in self.extents.iter().enumerate() {
            let at = 24 + index * 8;
            b[at..at + 4].copy_from_slice(&(run.start as u32).to_le_bytes());
            b[at + 4..at + 8].copy_from_slice(&(run.sectors as u32).to_le_bytes());
        }
        b[88..120].copy_from_slice(&self.name);
        b[120] = self.name_length;
        let checksum = crc(&b[..NODE_BYTES - 4]);
        b[NODE_BYTES - 4..].copy_from_slice(&checksum.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; NODE_BYTES]) -> Result<Self, Error> {
        let mut body = *b;
        let checksum = u32::from_le_bytes(body[NODE_BYTES - 4..].try_into().unwrap());
        body[NODE_BYTES - 4..].fill(0);
        if crc(&body[..NODE_BYTES - 4]) != checksum || b[121..124] != [0; 3] {
            return Err(Error::Corrupt);
        }
        let kind = match b[20] {
            0 => Kind::Empty,
            1 => Kind::File,
            2 => Kind::Directory,
            _ => return Err(Error::Corrupt),
        };
        let extents_used = b[22];
        if extents_used > EXTENTS_PER_FILE as u8 {
            return Err(Error::Corrupt);
        }
        if b[23] != 0 {
            return Err(Error::Corrupt);
        }
        let mut extents = [Extent::new(0, 0); EXTENTS_PER_FILE];
        for (index, run) in extents.iter_mut().enumerate() {
            let at = 24 + index * 8;
            let start = u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as u64;
            let sectors = u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap()) as u64;
            if start + sectors > DATA_SECTORS {
                return Err(Error::Corrupt);
            }
            *run = Extent::new(start, sectors);
        }
        for run in extents[extents_used as usize..].iter() {
            if run.sectors != 0 || run.start != 0 {
                return Err(Error::Corrupt);
            }
        }
        let length = u32::from_le_bytes(b[16..20].try_into().unwrap());
        let sectors: u64 = extents[..extents_used as usize]
            .iter()
            .map(|run| run.sectors)
            .sum();
        if u64::from(length) > sectors * SECTOR_BYTES {
            return Err(Error::Corrupt);
        }
        Ok(Self {
            id: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            parent: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            version: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            length,
            kind,
            space: b[21],
            extents,
            extents_used,
            name: b[88..120].try_into().unwrap(),
            name_length: b[120],
        })
    }

    pub fn runs(&self) -> &[Extent] {
        &self.extents[..self.extents_used as usize]
    }
}

/// The v6 header: region geometry, the sequence, and the checksums of the parts
/// that are stored outside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header6 {
    pub sequence: u64,
    pub objects: u32,
    pub nodes_checksum: u32,
    pub map_checksum: u32,
}

impl Header6 {
    pub const fn initial() -> Self {
        Self {
            sequence: 1,
            objects: OBJECTS_V6 as u32,
            nodes_checksum: 0,
            map_checksum: 0,
        }
    }
    pub fn encode(&self) -> [u8; SECTOR_BYTES as usize] {
        let mut b = [0; SECTOR_BYTES as usize];
        b[..8].copy_from_slice(&MAGIC);
        b[8] = VERSION;
        // Layout marker: the v5 header carries [0, 0, 2] here, and the v6 header
        // keeps the same position so a wrong layout is refused by inspection.
        b[9..12].copy_from_slice(&[0, 0, 2]);
        b[12..20].copy_from_slice(&self.sequence.to_le_bytes());
        b[20..24].copy_from_slice(&self.objects.to_le_bytes());
        b[24..28].copy_from_slice(&(NODE_BYTES as u32).to_le_bytes());
        b[28..32].copy_from_slice(&(DATA_SECTORS as u32).to_le_bytes());
        b[32..36].copy_from_slice(&self.nodes_checksum.to_le_bytes());
        b[36..40].copy_from_slice(&self.map_checksum.to_le_bytes());
        let checksum = crc(&b);
        b[40..44].copy_from_slice(&checksum.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8; SECTOR_BYTES as usize]) -> Result<Self, Error> {
        let mut body = *b;
        let checksum = u32::from_le_bytes(body[40..44].try_into().unwrap());
        body[40..44].fill(0);
        if b[..8] != MAGIC
            || b[8] != VERSION
            || b[9..12] != [0, 0, 2]
            || b[44..].iter().any(|byte| *byte != 0)
            || u32::from_le_bytes(b[20..24].try_into().unwrap()) as usize != OBJECTS_V6
            || u32::from_le_bytes(b[24..28].try_into().unwrap()) as usize != NODE_BYTES
            || u32::from_le_bytes(b[28..32].try_into().unwrap()) as u64 != DATA_SECTORS
            || crc(&body) != checksum
        {
            return Err(Error::Corrupt);
        }
        Ok(Self {
            sequence: u64::from_le_bytes(b[12..20].try_into().unwrap()),
            objects: u32::from_le_bytes(b[20..24].try_into().unwrap()),
            nodes_checksum: u32::from_le_bytes(b[32..36].try_into().unwrap()),
            map_checksum: u32::from_le_bytes(b[36..40].try_into().unwrap()),
        })
    }
}
