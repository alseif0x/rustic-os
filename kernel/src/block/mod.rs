// SPDX-License-Identifier: Apache-2.0
//! Internal sector contract, independent of transport and filesystem policy.
pub mod access;
pub mod observation;
pub mod queue;
pub const SECTOR: usize = 512;
pub const MAX_BYTES: usize = SECTOR;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Missing,
    Unsupported,
    Busy,
    Memory,
    Size,
    Range,
    ReadOnly,
    Io,
    Timeout,
    Protocol,
    Reset,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub sectors: u64,
    pub read_only: bool,
}
impl Geometry {
    pub fn validate(self, sector: u64, length: usize, write: bool) -> Result<(), Error> {
        if length != MAX_BYTES {
            return Err(Error::Size);
        }
        if sector
            .checked_add((length / SECTOR) as u64)
            .is_none_or(|end| end > self.sectors)
        {
            return Err(Error::Range);
        }
        if write && self.read_only {
            return Err(Error::ReadOnly);
        }
        Ok(())
    }
}
