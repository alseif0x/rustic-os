// SPDX-License-Identifier: Apache-2.0
use crate::{Error, MAX_FILE};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Empty = 0,
    File = 1,
    Directory = 2,
}
#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub id: u32,
    pub parent: u32,
    pub version: u64,
    pub length: u16,
    pub kind: Kind,
    pub space: u8,
    pub(super) bank: u8,
    pub(super) checksum: u32,
    name: [u8; 32],
    name_length: u8,
}
impl Node {
    pub const EMPTY: Self = Self {
        id: 0,
        parent: 0,
        version: 0,
        length: 0,
        kind: Kind::Empty,
        space: 0,
        bank: 0,
        checksum: 0,
        name: [0; 32],
        name_length: 0,
    };
    pub(super) fn new(
        id: u32,
        parent: u32,
        space: u8,
        kind: Kind,
        name: &[u8],
        version: u64,
    ) -> Result<Self, Error> {
        valid_name(name)?;
        let mut result = Self {
            id,
            parent,
            space,
            kind,
            version,
            ..Self::EMPTY
        };
        result.name[..name.len()].copy_from_slice(name);
        result.name_length = name.len() as u8;
        Ok(result)
    }
    pub fn name(&self) -> &[u8] {
        &self.name[..usize::from(self.name_length)]
    }
    pub(super) fn encode(self) -> [u8; 64] {
        let mut b = [0; 64];
        b[0] = self.kind as u8;
        b[1] = self.space;
        b[2] = self.bank;
        b[3] = self.name_length;
        b[4..8].copy_from_slice(&self.parent.to_le_bytes());
        b[8..12].copy_from_slice(&self.id.to_le_bytes());
        b[12..14].copy_from_slice(&self.length.to_le_bytes());
        b[16..24].copy_from_slice(&self.version.to_le_bytes());
        b[24..28].copy_from_slice(&self.checksum.to_le_bytes());
        b[32..].copy_from_slice(&self.name);
        b
    }
    pub(super) fn decode(b: &[u8]) -> Result<Self, Error> {
        if b.len() != 64 {
            return Err(Error::Corrupt);
        }
        if b == [0; 64] {
            return Ok(Self::EMPTY);
        }
        let kind = match b[0] {
            1 => Kind::File,
            2 => Kind::Directory,
            _ => return Err(Error::Corrupt),
        };
        let length = u16::from_le_bytes(b[12..14].try_into().unwrap());
        if !(1..=4).contains(&b[1])
            || b[2] > 1
            || !(1..=31).contains(&b[3])
            || usize::from(length) > MAX_FILE
            || b[14..16] != [0; 2]
            || b[28..32] != [0; 4]
            || b[32 + usize::from(b[3])..].iter().any(|b| *b != 0)
        {
            return Err(Error::Corrupt);
        }
        let mut node = Self::new(
            u32::from_le_bytes(b[8..12].try_into().unwrap()),
            u32::from_le_bytes(b[4..8].try_into().unwrap()),
            b[1],
            kind,
            &b[32..32 + usize::from(b[3])],
            u64::from_le_bytes(b[16..24].try_into().unwrap()),
        )
        .map_err(|_| Error::Corrupt)?;
        node.length = length;
        node.bank = b[2];
        node.checksum = u32::from_le_bytes(b[24..28].try_into().unwrap());
        if node.id == 0
            || node.version == 0
            || (kind == Kind::Directory
                && (node.length != 0 || node.checksum != 0 || node.bank != 0))
        {
            return Err(Error::Corrupt);
        }
        Ok(node)
    }
}
pub(super) fn valid_name(name: &[u8]) -> Result<(), Error> {
    if name.is_empty()
        || name.len() > 31
        || name == b"."
        || name == b".."
        || name
            .iter()
            .any(|b| !b.is_ascii_alphanumeric() && !b"._-".contains(b))
    {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
