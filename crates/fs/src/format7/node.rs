// SPDX-License-Identifier: Apache-2.0
//! The v7 control record: one object's identity, name and payload runs.
//!
//! Geometry is the v6 node, with the payload checksum moved to a fixed field:
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0 | 4 | id |
//! | 4 | 4 | parent |
//! | 8 | 8 | version |
//! | 16 | 4 | length |
//! | 20 | 1 | kind (1 file, 2 directory) |
//! | 21 | 1 | space (1..=4) |
//! | 22 | 1 | extents used |
//! | 23 | 1 | name length |
//! | 24 | 64 | eight `(start, sectors)` u32 pairs |
//! | 88 | 32 | name, zero padded |
//! | 120 | 4 | payload CRC-32 |
//! | 124 | 4 | CRC-32 over bytes 0..124 |
//!
//! This is record-local validation only. A node proves its own identity, name
//! and payload geometry; it cannot prove that its runs are free in the
//! generation's map, that its parent exists, or that its id is below the
//! watermark. Those need the volume, and no type here claims them.

use super::{MAX_EXTENTS, MAX_FILE_BYTES, NODE_BYTES, check_payload};
use crate::checksum::crc;
use crate::extent::Extent;
use crate::namespace::valid_name;
use crate::{Error, Kind};

/// Bytes of the fixed name field. A live name is shorter, and the tail is zero.
pub const NAME_BYTES: usize = 32;
/// Largest name the existing namespace rules accept.
pub const NAME_MAX: usize = 31;
/// Spaces a live object may belong to. Space 0 names no space.
pub const SPACES: core::ops::RangeInclusive<u8> = 1..=4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node7 {
    pub id: u32,
    pub parent: u32,
    pub version: u64,
    pub length: u32,
    pub kind: Kind,
    pub space: u8,
    pub extents_used: u8,
    pub extents: [Extent; MAX_EXTENTS],
    pub name_length: u8,
    pub name: [u8; NAME_BYTES],
    pub payload_crc32: u32,
}

impl Node7 {
    /// The canonical empty slot: every semantic field zero, serialized as 128
    /// zero bytes. A live object never has kind `Empty`.
    pub const EMPTY: Self = Self {
        id: 0,
        parent: 0,
        version: 0,
        length: 0,
        kind: Kind::Empty,
        space: 0,
        extents_used: 0,
        extents: [Extent::new(0, 0); MAX_EXTENTS],
        name_length: 0,
        name: [0; NAME_BYTES],
        payload_crc32: 0,
    };

    pub fn name(&self) -> &[u8] {
        &self.name[..usize::from(self.name_length)]
    }

    pub fn runs(&self) -> &[Extent] {
        &self.extents[..usize::from(self.extents_used)]
    }

    /// Serialize a record this codec would read back. Encoding refuses an invalid
    /// record instead of truncating a run into the 32-bit fields or persisting
    /// bytes its own decoder refuses.
    pub fn encode(&self) -> Result<[u8; NODE_BYTES], Error> {
        self.validate()?;
        if *self == Self::EMPTY {
            return Ok([0; NODE_BYTES]);
        }
        let mut b = [0; NODE_BYTES];
        b[0..4].copy_from_slice(&self.id.to_le_bytes());
        b[4..8].copy_from_slice(&self.parent.to_le_bytes());
        b[8..16].copy_from_slice(&self.version.to_le_bytes());
        b[16..20].copy_from_slice(&self.length.to_le_bytes());
        b[20] = self.kind as u8;
        b[21] = self.space;
        b[22] = self.extents_used;
        b[23] = self.name_length;
        for (index, run) in self.extents.iter().enumerate() {
            let at = 24 + index * 8;
            b[at..at + 4].copy_from_slice(&(run.start as u32).to_le_bytes());
            b[at + 4..at + 8].copy_from_slice(&(run.sectors as u32).to_le_bytes());
        }
        b[88..120].copy_from_slice(&self.name);
        b[120..124].copy_from_slice(&self.payload_crc32.to_le_bytes());
        let checksum = crc(&b[..124]);
        b[124..].copy_from_slice(&checksum.to_le_bytes());
        Ok(b)
    }

    /// Strict record decoding: the checksum, the kind byte and every reserved or
    /// unused byte must hold, and the record's own invariants must pass. An
    /// all-zero slot is the canonical empty record; a live object is never empty.
    pub fn decode(b: &[u8; NODE_BYTES]) -> Result<Self, Error> {
        if b == &[0; NODE_BYTES] {
            return Ok(Self::EMPTY);
        }
        let checksum = u32::from_le_bytes(b[124..128].try_into().unwrap());
        if crc(&b[..124]) != checksum {
            return Err(Error::Corrupt);
        }
        let mut extents = [Extent::new(0, 0); MAX_EXTENTS];
        for (index, run) in extents.iter_mut().enumerate() {
            let at = 24 + index * 8;
            *run = Extent::new(
                u64::from(u32::from_le_bytes(b[at..at + 4].try_into().unwrap())),
                u64::from(u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap())),
            );
        }
        let node = Self {
            id: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            parent: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            version: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            length: u32::from_le_bytes(b[16..20].try_into().unwrap()),
            kind: match b[20] {
                1 => Kind::File,
                2 => Kind::Directory,
                _ => return Err(Error::Corrupt),
            },
            space: b[21],
            extents_used: b[22],
            extents,
            name_length: b[23],
            name: b[88..120].try_into().unwrap(),
            payload_crc32: u32::from_le_bytes(b[120..124].try_into().unwrap()),
        };
        node.validate()?;
        Ok(node)
    }

    /// What one record can prove about itself: a live identity in a real space,
    /// a valid zero-padded name, and payload runs that describe the length
    /// exactly. A directory owns no payload; a file may be empty.
    pub fn validate(&self) -> Result<(), Error> {
        if *self == Self::EMPTY {
            return Ok(());
        }
        if self.id == 0
            || self.version == 0
            || self.kind == Kind::Empty
            || !SPACES.contains(&self.space)
            || self.name_length as usize > NAME_MAX
        {
            return Err(Error::Corrupt);
        }
        if valid_name(self.name()).is_err() {
            return Err(Error::Corrupt);
        }
        if self.name[usize::from(self.name_length)..]
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err(Error::Corrupt);
        }
        if self.kind == Kind::Directory
            && (self.length != 0 || self.payload_crc32 != 0 || self.extents_used != 0)
        {
            return Err(Error::Corrupt);
        }
        if u64::from(self.length) > u64::from(MAX_FILE_BYTES) {
            return Err(Error::Corrupt);
        }
        check_payload(&self.extents, self.extents_used, self.length)
    }
}
