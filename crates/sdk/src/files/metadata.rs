// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{Error, Packet};
#[derive(Clone, Copy, Debug)]
pub struct Metadata {
    pub id: u32,
    pub parent: u32,
    pub version: u64,
    pub length: usize,
    pub directory: bool,
    pub space: u8,
    pub cursor: u8,
    name: [u8; 32],
    name_length: usize,
}
impl Metadata {
    pub(super) fn decode(p: Packet) -> Result<Self, Error> {
        let n = p.data[2] as usize;
        if p.count != 40 || !(1..=2).contains(&p.data[0]) || n > 31 || p.id == 0 {
            return Err(Error::Protocol);
        }
        let mut name = [0; 32];
        name.copy_from_slice(&p.data[8..40]);
        Ok(Self {
            id: p.id,
            parent: u32::from_le_bytes(p.data[4..8].try_into().unwrap()),
            version: p.version,
            length: p.arg as usize,
            directory: p.data[0] == 2,
            space: p.data[1],
            cursor: p.data[3],
            name,
            name_length: n,
        })
    }
    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_length]).unwrap_or("?")
    }
}
