// SPDX-License-Identifier: Apache-2.0
use super::Error;
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub size: usize,
    pub available: usize,
    pub used: usize,
    pub pages: usize,
}
impl Layout {
    pub fn new(size: usize) -> Result<Self, Error> {
        if !(4..=256).contains(&size) || !size.is_power_of_two() {
            return Err(Error::Unsupported);
        }
        let available = size * 16;
        let used = (available + 6 + size * 2).div_ceil(4096) * 4096;
        let pages = (used + 6 + size * 8).div_ceil(4096);
        Ok(Self {
            size,
            available,
            used,
            pages,
        })
    }
    pub fn completed(expected: u16, observed: u16, id: u32, status: u8) -> Result<(), Error> {
        if observed != expected.wrapping_add(1) || id != 0 {
            return Err(Error::Protocol);
        }
        match status {
            0 => Ok(()),
            1 => Err(Error::Io),
            2 => Err(Error::Unsupported),
            _ => Err(Error::Protocol),
        }
    }
}
