// SPDX-License-Identifier: Apache-2.0
//! Additive INT 0x80 extension. All wire integers are little endian.
pub const SEND: u64 = 4;
pub const RECEIVE: u64 = 5;
pub const WAIT: u64 = 6;
pub const CLOSE: u64 = 7;
pub const INFO: u64 = 8;
pub const VERSION: u16 = 1;
pub const DATA: u16 = 1;
pub const HEADER: usize = 24;
/// One data-carrying operation should move a bounded file object, not 40 bytes
/// (#50). The message grows so the "one logical call = one message" model holds;
/// every queue entry pays this size.
pub const PAYLOAD: usize = 1024;
pub const MAX_MESSAGE: usize = HEADER + PAYLOAD;
pub const READ: u8 = 1;
pub const WRITE: u8 = 2;
pub const TRANSFER: u8 = 4;
pub const ALL: u8 = READ | WRITE | TRANSFER;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Handle,
    Denied,
    Address,
    Size,
    Version,
    Message,
    WouldBlock,
    Closed,
    Cancelled,
    Quota,
}
impl Error {
    pub const fn code(self) -> u64 {
        u64::MAX
            - match self {
                Self::Handle => 2,
                Self::Denied => 3,
                Self::Address => 4,
                Self::Size => 5,
                Self::Version => 6,
                Self::Message => 7,
                Self::WouldBlock => 8,
                Self::Closed => 9,
                Self::Cancelled => 10,
                Self::Quota => 11,
            }
    }
}
