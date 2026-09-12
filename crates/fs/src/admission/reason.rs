// SPDX-License-Identifier: Apache-2.0
//! A retained prevention cause; service policy selects it, storage preserves it.
use crate::{Disk, Error, Volume};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PreventionReason {
    /// Format v4 did not record a cause. Migration cannot reconstruct one.
    Unknown = 0,
    Requested = 1,
    VersionConflict = 2,
    AuthorityLost = 3,
}

impl Volume {
    pub fn prevention_reasons_enabled(&self) -> Result<bool, Error> {
        Ok(self.admissions()?.reasons)
    }

    /// Explicit one-way v4 -> v5 migration. Preserve records and finish both banks
    /// before success so a v4 reader cannot select a stale pre-migration fallback.
    /// After uncertainty, remount and explicitly retry; never upgrade on mount.
    pub fn enable_prevention_reasons(&mut self, disk: &mut impl Disk) -> Result<(), Error> {
        self.admissions()?;
        let mut next = self.metadata.clone();
        let recovery = next.recovery.as_mut().unwrap();
        if !recovery.reasons {
            recovery.reasons = true;
            self.commit(disk, next)?;
        }
        match crate::format::Metadata::read(disk, 1 - self.bank) {
            Ok(other) if other.recovery.as_ref().is_some_and(|r| r.reasons) => Ok(()),
            Ok(_) | Err(Error::Corrupt | Error::Empty) => self.commit(disk, self.metadata.clone()),
            Err(_) => {
                self.poisoned = true;
                Err(Error::Uncertain)
            }
        }
    }
}
