// SPDX-License-Identifier: Apache-2.0
//! Native runtime extension 1, explicit integer arrays and disjoint error range.
pub const VERSION: u64 = 1;
pub const CLOCK: u64 = 15;
pub const CONSOLE_WRITE: u64 = 16;
pub const CONSOLE_READ: u64 = 17;
pub const CONSOLE_WAIT: u64 = 18;
pub const WAIT_SET: u64 = 19;
pub const CONTROL: u64 = 20;
pub const INFO: u64 = 0;
pub const SPAWN: u64 = 1;
pub const START: u64 = 2;
pub const CONNECT: u64 = 3;
pub const BLOCK_GRANT: u64 = 4;
pub const CONSOLE_GRANT: u64 = 5;
pub const PROCESS: u64 = 6;
pub const KILL: u64 = 7;
pub const REAP: u64 = 8;
pub const SHUTDOWN: u64 = 9;
pub const DEVICE: u64 = 11;
pub const CLOSE_ENDPOINT: u64 = 12;
pub const FILES: u64 = 1;
pub const SHELL: u64 = 2;
pub const UTILITY: u64 = 3;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Error {
    Denied = 0,
    Address = 1,
    Size = 2,
    Invalid = 3,
    Busy = 4,
    NotFound = 5,
    Full = 6,
    WouldBlock = 7,
    Closed = 8,
    Protocol = 9,
}
impl Error {
    pub const fn code(self) -> u64 {
        u64::MAX - 64 - self as u64
    }
    pub fn decode(value: u64) -> Result<u64, Self> {
        for e in [
            Self::Denied,
            Self::Address,
            Self::Size,
            Self::Invalid,
            Self::Busy,
            Self::NotFound,
            Self::Full,
            Self::WouldBlock,
            Self::Closed,
            Self::Protocol,
        ] {
            if value == e.code() {
                return Err(e);
            }
        }
        if value > u64::MAX - 4096 {
            Err(Self::Protocol)
        } else {
            Ok(value)
        }
    }
}
/// Strict shape validation is shared with host contract tests.
pub fn validate_control(w: [u64; 8]) -> Result<(), Error> {
    let end = match w[0] {
        INFO | SHUTDOWN | DEVICE => 1,
        SPAWN | CONSOLE_GRANT | PROCESS | KILL | REAP => 2,
        CONNECT | CLOSE_ENDPOINT => 3,
        START | BLOCK_GRANT => 5,
        _ => return Err(Error::Invalid),
    };
    if w[end..].iter().any(|v| *v != 0) {
        return Err(Error::Invalid);
    }
    Ok(())
}
pub fn encode(words: [u64; 8]) -> [u8; 64] {
    let mut bytes = [0; 64];
    for (word, b) in words.iter().zip(bytes.as_chunks_mut::<8>().0) {
        b.copy_from_slice(&word.to_le_bytes());
    }
    bytes
}
pub fn decode(bytes: &[u8]) -> Result<[u64; 8], Error> {
    if bytes.len() != 64 {
        return Err(Error::Size);
    }
    let mut words = [0; 8];
    for (word, b) in words.iter_mut().zip(bytes.as_chunks::<8>().0) {
        *word = u64::from_le_bytes(*b);
    }
    Ok(words)
}
