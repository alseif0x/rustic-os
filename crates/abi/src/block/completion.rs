// SPDX-License-Identifier: Apache-2.0
use super::{Error, MAX_ID, Operation, RESULT_BYTES, SECTOR, VERSION, wire::*};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Status {
    Success,
    Cancelled,
    Io,
    Timeout,
    Protocol,
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Effect {
    None,
    Completed,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Completion {
    pub id: u64,
    pub operation: Operation,
    pub status: Status,
    pub effect: Effect,
    pub data: [u8; SECTOR],
}
impl Completion {
    pub fn length(&self) -> usize {
        if self.status == Status::Success && self.operation == Operation::Read {
            SECTOR
        } else {
            0
        }
    }
    pub fn encode(&self) -> [u8; RESULT_BYTES] {
        let mut bytes = [0; RESULT_BYTES];
        bytes[..2].copy_from_slice(&VERSION.to_le_bytes());
        bytes[2..4].copy_from_slice(&(self.operation as u16).to_le_bytes());
        bytes[4..8].copy_from_slice(&(self.status as u32).to_le_bytes());
        bytes[8..16].copy_from_slice(&self.id.to_le_bytes());
        bytes[16..20].copy_from_slice(&(self.length() as u32).to_le_bytes());
        bytes[20..24].copy_from_slice(&(self.effect as u32).to_le_bytes());
        bytes[32..32 + self.length()].copy_from_slice(&self.data[..self.length()]);
        bytes
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != RESULT_BYTES {
            return Err(Error::Size);
        }
        if u16_at(bytes, 0) != VERSION {
            return Err(Error::Version);
        }
        let operation = Operation::parse(u16_at(bytes, 2))?;
        let status = match u32_at(bytes, 4) {
            0 => Status::Success,
            1 => Status::Cancelled,
            2 => Status::Io,
            3 => Status::Timeout,
            4 => Status::Protocol,
            5 => Status::Unavailable,
            _ => return Err(Error::Protocol),
        };
        let effect = match u32_at(bytes, 20) {
            0 => Effect::None,
            1 => Effect::Completed,
            2 => Effect::Unknown,
            _ => return Err(Error::Protocol),
        };
        let id = u64_at(bytes, 8);
        let expected = if status == Status::Success && operation == Operation::Read {
            SECTOR
        } else {
            0
        };
        let valid_effect = if operation == Operation::Read
            || status == Status::Cancelled
            || status == Status::Unavailable
        {
            effect == Effect::None
        } else if status == Status::Success {
            effect == Effect::Completed
        } else {
            effect == Effect::Unknown
        };
        if id == 0
            || id > MAX_ID
            || u32_at(bytes, 16) as usize != expected
            || u64_at(bytes, 24) != 0
            || bytes[32 + expected..].iter().any(|b| *b != 0)
            || !valid_effect
        {
            return Err(Error::Protocol);
        }
        let mut data = [0; SECTOR];
        data[..expected].copy_from_slice(&bytes[32..32 + expected]);
        Ok(Self {
            id,
            operation,
            status,
            effect,
            data,
        })
    }
}
