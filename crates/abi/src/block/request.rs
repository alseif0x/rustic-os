// SPDX-License-Identifier: Apache-2.0
use super::{Error, REQUEST_BYTES, SECTOR, VERSION, wire::*};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Operation {
    Read = 1,
    Write = 2,
    Flush = 3,
}
impl Operation {
    pub fn parse(value: u16) -> Result<Self, Error> {
        match value {
            1 => Ok(Self::Read),
            2 => Ok(Self::Write),
            3 => Ok(Self::Flush),
            _ => Err(Error::Request),
        }
    }
    pub const fn right(self) -> u8 {
        match self {
            Self::Read => super::READ,
            Self::Write => super::WRITE,
            Self::Flush => super::FLUSH,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub operation: Operation,
    pub sector: u64,
    pub address: u64,
    pub length: u32,
}
impl Request {
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != REQUEST_BYTES {
            return Err(Error::Size);
        }
        if u16_at(bytes, 0) != VERSION {
            return Err(Error::Version);
        }
        if u32_at(bytes, 4) != 0 || u32_at(bytes, 28) != 0 {
            return Err(Error::Request);
        }
        let request = Self {
            operation: Operation::parse(u16_at(bytes, 2))?,
            sector: u64_at(bytes, 8),
            address: u64_at(bytes, 16),
            length: u32_at(bytes, 24),
        };
        match request.operation {
            Operation::Flush
                if request.sector != 0 || request.address != 0 || request.length != 0 =>
            {
                return Err(Error::Request);
            }
            Operation::Read | Operation::Write if request.length != SECTOR as u32 => {
                return Err(Error::Size);
            }
            Operation::Read if request.address != 0 => return Err(Error::Request),
            _ => {}
        }
        Ok(request)
    }
    pub fn encode(self) -> [u8; REQUEST_BYTES] {
        let mut bytes = [0; REQUEST_BYTES];
        bytes[..2].copy_from_slice(&VERSION.to_le_bytes());
        bytes[2..4].copy_from_slice(&(self.operation as u16).to_le_bytes());
        bytes[8..16].copy_from_slice(&self.sector.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.address.to_le_bytes());
        bytes[24..28].copy_from_slice(&self.length.to_le_bytes());
        bytes
    }
}
