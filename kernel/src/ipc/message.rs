// SPDX-License-Identifier: Apache-2.0
use rustic_abi::ipc::{DATA, Error, HEADER, MAX_MESSAGE, PAYLOAD, VERSION};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    correlation: u64,
    sender: u64,
    payload: [u8; PAYLOAD],
    len: usize,
}
impl Message {
    pub fn decode(bytes: &[u8], sender: u64) -> Result<Self, Error> {
        if !(HEADER..=MAX_MESSAGE).contains(&bytes.len()) {
            return Err(Error::Size);
        }
        if u16::from_le_bytes(bytes[..2].try_into().unwrap()) != VERSION {
            return Err(Error::Version);
        }
        if u16::from_le_bytes(bytes[2..4].try_into().unwrap()) != DATA
            || bytes[16..24].iter().any(|b| *b != 0)
        {
            return Err(Error::Message);
        }
        let len = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        if len > PAYLOAD || bytes.len() != HEADER + len {
            return Err(Error::Size);
        }
        let mut payload = [0; PAYLOAD];
        payload[..len].copy_from_slice(&bytes[HEADER..]);
        Ok(Self {
            correlation: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            sender,
            payload,
            len,
        })
    }
    pub fn length(&self) -> usize {
        HEADER + self.len
    }
    pub fn encode(&self) -> [u8; MAX_MESSAGE] {
        let mut bytes = [0; MAX_MESSAGE];
        bytes[..2].copy_from_slice(&VERSION.to_le_bytes());
        bytes[2..4].copy_from_slice(&DATA.to_le_bytes());
        bytes[4..8].copy_from_slice(&(self.len as u32).to_le_bytes());
        bytes[8..16].copy_from_slice(&self.correlation.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.sender.to_le_bytes());
        bytes[HEADER..HEADER + self.len].copy_from_slice(&self.payload[..self.len]);
        bytes
    }
}
