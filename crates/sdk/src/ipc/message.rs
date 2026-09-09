// SPDX-License-Identifier: Apache-2.0
use crate::Error;
use rustic_abi::ipc as abi;
/// Owned fixed storage; no wire casts or heap allocation.
pub struct Message {
    bytes: [u8; abi::MAX_MESSAGE],
    length: usize,
}
impl Message {
    pub fn new(correlation: u64, payload: &[u8]) -> Result<Self, Error> {
        if payload.len() > abi::PAYLOAD {
            return Err(Error::Ipc(abi::Error::Size));
        }
        let mut bytes = [0; abi::MAX_MESSAGE];
        bytes[..2].copy_from_slice(&abi::VERSION.to_le_bytes());
        bytes[2..4].copy_from_slice(&abi::DATA.to_le_bytes());
        bytes[4..8].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes[8..16].copy_from_slice(&correlation.to_le_bytes());
        bytes[abi::HEADER..abi::HEADER + payload.len()].copy_from_slice(payload);
        Ok(Self {
            bytes,
            length: abi::HEADER + payload.len(),
        })
    }
    pub fn from_received(bytes: &[u8]) -> Result<Self, Error> {
        if !(abi::HEADER..=abi::MAX_MESSAGE).contains(&bytes.len()) {
            return Err(Error::Protocol);
        }
        let version = u16::from_le_bytes(bytes[..2].try_into().unwrap());
        let opcode = u16::from_le_bytes(bytes[2..4].try_into().unwrap());
        let length = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        if version != abi::VERSION
            || opcode != abi::DATA
            || length != bytes.len() - abi::HEADER
            || bytes[16..24] == [0; 8]
        {
            return Err(Error::Protocol);
        }
        let mut result = Self::new(
            u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            &bytes[24..],
        )?;
        result.bytes[16..24].copy_from_slice(&bytes[16..24]);
        Ok(result)
    }
    pub fn correlation(&self) -> u64 {
        u64::from_le_bytes(self.bytes[8..16].try_into().unwrap())
    }
    pub fn sender(&self) -> u64 {
        u64::from_le_bytes(self.bytes[16..24].try_into().unwrap())
    }
    pub fn payload(&self) -> &[u8] {
        &self.bytes[abi::HEADER..self.length]
    }
    pub fn wire(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}
