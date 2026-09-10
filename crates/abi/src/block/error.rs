// SPDX-License-Identifier: Apache-2.0
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Error {
    Handle,
    Denied,
    Address,
    Size,
    Version,
    Request,
    Busy,
    Quota,
    Range,
    ReadOnly,
    Unavailable,
    NoRequest,
    WouldBlock,
    Protocol,
}
impl Error {
    pub const fn code(self) -> u64 {
        u64::MAX - 32 - self as u64
    }
    pub fn decode(value: u64) -> Result<u64, Self> {
        for error in [
            Self::Handle,
            Self::Denied,
            Self::Address,
            Self::Size,
            Self::Version,
            Self::Request,
            Self::Busy,
            Self::Quota,
            Self::Range,
            Self::ReadOnly,
            Self::Unavailable,
            Self::NoRequest,
            Self::WouldBlock,
            Self::Protocol,
        ] {
            if value == error.code() {
                return Err(error);
            }
        }
        if value > super::MAX_ID {
            Err(Self::Protocol)
        } else {
            Ok(value)
        }
    }
}
