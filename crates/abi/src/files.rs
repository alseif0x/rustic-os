// SPDX-License-Identifier: Apache-2.0
//! Native file-service wire contract, independent from filesystem representation.
pub const VERSION: u8 = 1;
pub const SIZE: usize = 64;
pub const DATA: usize = 40;
pub const LOOKUP: u8 = 1;
pub const STAT: u8 = 2;
pub const LIST: u8 = 3;
pub const CREATE: u8 = 4;
pub const REMOVE: u8 = 5;
pub const READ: u8 = 6;
pub const BEGIN: u8 = 7;
pub const CHUNK: u8 = 8;
pub const COMMIT: u8 = 9;
pub const ABORT: u8 = 10;
pub const MKDIR: u8 = 11;
pub const RECOVERY: u8 = 12;
pub const TRACK_BEGIN: u8 = 13;
pub const RECEIPT: u8 = 14;
pub const REFERENCES: u8 = 15;
pub const READ_OPEN: u8 = 16;
pub const READ_CHUNK: u8 = 17;
pub const REPLACE_OPEN: u8 = 18;
pub const REPLACE_CHUNK: u8 = 19;
pub const REPLACE_COMMIT: u8 = 20;
pub const REPLACE_ABORT: u8 = 21;
pub const OPERATION_RETRY: u8 = 22;
pub const OPERATION_ID: u8 = 23;
pub const OPERATION_PART: u8 = 24;
/// Bounded discovery of what this service implements; carries no authority.
pub const CAPABILITIES: u8 = 58;
/// Largest inline payload any logical method accepts in this profile.
pub const MAX_INLINE: usize = 1024;
pub mod admission;
pub mod capabilities;
pub const CANCEL_RIGHT: u8 = 8;
pub mod operation;
pub const INSPECT_RIGHT: u8 = 4;
pub mod read;
pub mod recovery;
pub mod reference;
pub const GRANT: u8 = 32;
pub const REVOKE: u8 = 33;
pub const STATUS: u8 = 34;
pub const READ_RIGHT: u8 = 1;
pub const WRITE_RIGHT: u8 = 2;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Error {
    Protocol = 1,
    Io = 2,
    Uncertain = 3,
    Invalid = 4,
    Corrupt = 5,
    NotFound = 6,
    Exists = 7,
    NotDirectory = 8,
    IsDirectory = 9,
    NotEmpty = 10,
    Full = 11,
    Size = 12,
    Version = 13,
    ReadOnly = 14,
    Empty = 15,
    Exhausted = 16,
    Denied = 17,
    Revoked = 18,
    Expired = 19,
    Busy = 20,
    NoTransfer = 21,
    Offset = 22,
    Closed = 23,
    Unsupported = 24,
    Lineage = 25,
    ExpiredEpoch = 26,
    OutcomeUnknown = 27,
    IdempotencyConflict = 28,
    Interrupted = 29,
    UnsupportedVersion = 30,
    Unavailable = 31,
}
impl Error {
    pub fn parse(value: u8) -> Result<(), Self> {
        Err(match value {
            0 => return Ok(()),
            1 => Self::Protocol,
            2 => Self::Io,
            3 => Self::Uncertain,
            4 => Self::Invalid,
            5 => Self::Corrupt,
            6 => Self::NotFound,
            7 => Self::Exists,
            8 => Self::NotDirectory,
            9 => Self::IsDirectory,
            10 => Self::NotEmpty,
            11 => Self::Full,
            12 => Self::Size,
            13 => Self::Version,
            14 => Self::ReadOnly,
            15 => Self::Empty,
            16 => Self::Exhausted,
            17 => Self::Denied,
            18 => Self::Revoked,
            19 => Self::Expired,
            20 => Self::Busy,
            21 => Self::NoTransfer,
            22 => Self::Offset,
            23 => Self::Closed,
            24 => Self::Unsupported,
            25 => Self::Lineage,
            26 => Self::ExpiredEpoch,
            27 => Self::OutcomeUnknown,
            28 => Self::IdempotencyConflict,
            29 => Self::Interrupted,
            30 => Self::UnsupportedVersion,
            31 => Self::Unavailable,
            _ => Self::Protocol,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet {
    pub op: u8,
    pub status: u8,
    pub count: u8,
    pub id: u32,
    pub arg: u32,
    pub context: u32,
    pub version: u64,
    pub data: [u8; DATA],
}
impl Packet {
    pub const fn new(op: u8) -> Self {
        Self {
            op,
            status: 0,
            count: 0,
            id: 0,
            arg: 0,
            context: 0,
            version: 0,
            data: [0; DATA],
        }
    }
    pub fn encode(&self) -> [u8; SIZE] {
        let mut b = [0; SIZE];
        b[0] = VERSION;
        b[1] = self.op;
        b[2] = self.status;
        b[3] = self.count;
        b[4..8].copy_from_slice(&self.id.to_le_bytes());
        b[8..12].copy_from_slice(&self.arg.to_le_bytes());
        b[12..16].copy_from_slice(&self.context.to_le_bytes());
        b[16..24].copy_from_slice(&self.version.to_le_bytes());
        b[24..].copy_from_slice(&self.data);
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self, Error> {
        if b.len() != SIZE
            || b[0] != VERSION
            || b[3] as usize > DATA
            || !matches!(b[1],1..=24|32..=34|48..=60)
        {
            return Err(Error::Protocol);
        }
        let mut data = [0; DATA];
        data.copy_from_slice(&b[24..]);
        if data[usize::from(b[3])..].iter().any(|b| *b != 0) {
            return Err(Error::Protocol);
        }
        Ok(Self {
            op: b[1],
            status: b[2],
            count: b[3],
            id: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            arg: u32::from_le_bytes(b[8..12].try_into().unwrap()),
            context: u32::from_le_bytes(b[12..16].try_into().unwrap()),
            version: u64::from_le_bytes(b[16..24].try_into().unwrap()),
            data,
        })
    }
    pub fn payload(&self) -> &[u8] {
        &self.data[..usize::from(self.count).min(DATA)]
    }
    /// Validate a complete reply after IPC has authenticated its peer/correlation.
    /// Error replies carry no leftover result fields or data.
    pub fn checked_reply(self, op: u8, context: u32) -> Result<Self, Error> {
        if self.op != op
            || self.context != context
            || self.count as usize > DATA
            || self.data[usize::from(self.count)..].iter().any(|b| *b != 0)
            || self.status != 0
                && (self.id != 0 || self.arg != 0 || self.version != 0 || self.count != 0)
        {
            return Err(Error::Protocol);
        }
        Error::parse(self.status)?;
        Ok(self)
    }
}
