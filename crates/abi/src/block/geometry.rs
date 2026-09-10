// SPDX-License-Identifier: Apache-2.0
use super::{ALL, Error, GEOMETRY_BYTES, SECTOR, VERSION, wire::*};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub sectors: u64,
    pub rights: u8,
    pub read_only: bool,
}
impl Geometry {
    pub fn encode(self) -> [u8; GEOMETRY_BYTES] {
        let mut bytes = [0; GEOMETRY_BYTES];
        bytes[..2].copy_from_slice(&VERSION.to_le_bytes());
        bytes[2..4].copy_from_slice(&(GEOMETRY_BYTES as u16).to_le_bytes());
        bytes[4..8].copy_from_slice(&u32::from(self.rights).to_le_bytes());
        bytes[8..16].copy_from_slice(&self.sectors.to_le_bytes());
        bytes[16..20].copy_from_slice(&(SECTOR as u32).to_le_bytes());
        bytes[20..24].copy_from_slice(&(SECTOR as u32).to_le_bytes());
        bytes[24..28].copy_from_slice(&u32::from(self.read_only).to_le_bytes());
        bytes
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != GEOMETRY_BYTES {
            return Err(Error::Size);
        }
        if u16_at(bytes, 0) != VERSION {
            return Err(Error::Version);
        }
        let rights = u32_at(bytes, 4);
        if u16_at(bytes, 2) != GEOMETRY_BYTES as u16
            || rights == 0
            || rights & !u32::from(ALL) != 0
            || u64_at(bytes, 8) == 0
            || u32_at(bytes, 16) != SECTOR as u32
            || u32_at(bytes, 20) != SECTOR as u32
            || u32_at(bytes, 24) > 1
            || u32_at(bytes, 28) != 0
        {
            return Err(Error::Protocol);
        }
        Ok(Self {
            sectors: u64_at(bytes, 8),
            rights: rights as u8,
            read_only: u32_at(bytes, 24) != 0,
        })
    }
}
