// SPDX-License-Identifier: Apache-2.0
//! Native bounded recovery binding; not the logical JSON service-v1 surface.
use super::{Error, Packet};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retry {
    pub lineage: [u8; 16],
    pub epoch: u64,
    pub key: u64,
}
impl Retry {
    pub fn encode(self) -> [u8; 32] {
        let mut b = [0; 32];
        b[..16].copy_from_slice(&self.lineage);
        b[16..24].copy_from_slice(&self.epoch.to_le_bytes());
        b[24..32].copy_from_slice(&self.key.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self, Error> {
        if b.len() != 32 {
            return Err(Error::Protocol);
        }
        let value = Self {
            lineage: b[..16].try_into().unwrap(),
            epoch: u64::from_le_bytes(b[16..24].try_into().unwrap()),
            key: u64::from_le_bytes(b[24..32].try_into().unwrap()),
        };
        if value.lineage == [0; 16] || value.epoch == 0 || value.key == 0 {
            return Err(Error::Invalid);
        }
        Ok(value)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub retry: Retry,
    pub id: u32,
    pub previous: u64,
    pub committed: u64,
    pub length: u16,
}
impl Receipt {
    pub fn packet(self, mut p: Packet) -> Packet {
        p.id = self.id;
        p.version = self.committed;
        p.arg = self.length as u32;
        p.count = 40;
        p.data[..32].copy_from_slice(&self.retry.encode());
        p.data[32..].copy_from_slice(&self.previous.to_le_bytes());
        p
    }
    pub fn decode(p: Packet) -> Result<Self, Error> {
        if p.count != 40 || p.id <= 4 || p.arg > 1024 {
            return Err(Error::Protocol);
        }
        let previous = u64::from_le_bytes(p.data[32..].try_into().unwrap());
        if previous == 0 || p.version <= previous {
            return Err(Error::Protocol);
        }
        Ok(Self {
            retry: Retry::decode(&p.data[..32])?,
            id: p.id,
            previous,
            committed: p.version,
            length: p.arg as u16,
        })
    }
}
