// SPDX-License-Identifier: Apache-2.0
use rustic_abi::block::{Error, FLUSH};
#[derive(Clone, Copy, Debug)]
pub struct Grant {
    pub first: u64,
    pub sectors: u64,
    pub rights: u8,
}
impl Grant {
    pub(super) fn validate(self, capacity: u64) -> Result<(), Error> {
        if self.sectors == 0
            || self
                .first
                .checked_add(self.sectors)
                .is_none_or(|end| end > capacity)
        {
            return Err(Error::Range);
        }
        if self.rights & FLUSH != 0 && (self.first != 0 || self.sectors != capacity) {
            return Err(Error::Denied);
        }
        Ok(())
    }
    pub(super) fn sector(self, relative: u64) -> Result<u64, Error> {
        if relative >= self.sectors {
            return Err(Error::Range);
        }
        self.first.checked_add(relative).ok_or(Error::Range)
    }
}
