// SPDX-License-Identifier: Apache-2.0
use crate::{Disk, Error, Volume};
impl Volume {
    /// Owner-requested one-way v4 migration; preserves all existing receipts.
    /// Complete both banks so an older reader cannot select a stale v3 fallback.
    pub fn enable_admissions(&mut self, disk: &mut impl Disk) -> Result<(), Error> {
        self.ready()?;
        let mut next = self.metadata.clone();
        let r = next
            .recovery
            .as_mut()
            .filter(|r| r.scoped)
            .ok_or(Error::Unsupported)?;
        if !r.admissions {
            r.admissions = true;
            self.commit(disk, next)?;
        }
        match crate::format::Metadata::read(disk, 1 - self.bank) {
            Ok(other) if other.recovery.as_ref().is_some_and(|r| r.admissions) => Ok(()),
            Ok(_) | Err(Error::Corrupt | Error::Empty) => self.commit(disk, self.metadata.clone()),
            Err(_) => {
                self.poisoned = true;
                Err(Error::Uncertain)
            }
        }
    }
}
