// SPDX-License-Identifier: Apache-2.0
//! Copy the bounded retained admission table and verified scope proofs.
use super::{Candidate7, Scheduler7};
use crate::v7::{
    admission::scope::{self, Scope7},
    grants::Grants,
};
use rustic_abi::files::Error;
use rustic_fs::Volume7;

impl Scheduler7 {
    pub(in crate::v7) fn refresh(
        &mut self,
        volume: &Volume7,
        grants: &Grants,
    ) -> Result<(), Error> {
        self.candidates.fill(None);
        let records = match volume.retained_records() {
            Ok(records) => records,
            Err(error) => {
                self.clear();
                return Err(crate::reply::error(error));
            }
        };
        for (index, record) in records.iter().enumerate() {
            let Some(record) = *record else {
                continue;
            };
            if !scope::is_admission(&record) {
                continue;
            }
            self.candidates[index] = Some(Candidate7 {
                scope: Scope7::capture(volume, grants, record)?,
                record,
            });
        }
        Ok(())
    }
}
