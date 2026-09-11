// SPDX-License-Identifier: Apache-2.0
use super::{Admission, AdmissionId, AdmissionState, AdmissionStatus};
use crate::{
    Error, Replacement, Retry, Volume,
    recovery::{Record, Recovery},
};

impl Record {
    pub(crate) fn admission_view(&self) -> Result<Admission<'_>, Error> {
        let a = self.admission.ok_or(Error::Unsupported)?;
        let (workspace, instance) = self.namespace.ok_or(Error::Corrupt)?;
        Ok(Admission {
            status: AdmissionStatus {
                id: AdmissionId {
                    lineage: self.receipt.retry.lineage,
                    number: a.number,
                },
                state: a.state,
                terminal: a.terminal,
            },
            request: Replacement {
                workspace,
                retry: self.receipt.retry,
                id: self.receipt.id,
                version: self.receipt.previous,
            },
            instance,
            receipt: (a.state == AdmissionState::Committed).then_some(self.receipt),
            bytes: &self.bytes[..usize::from(self.receipt.length)],
        })
    }
}
impl Volume {
    pub(super) fn admissions(&self) -> Result<&Recovery, Error> {
        self.ready()?;
        self.metadata
            .recovery
            .as_ref()
            .filter(|r| r.admissions)
            .ok_or(Error::Unsupported)
    }

    pub(super) fn admission_slot(&self, subject: u64, id: AdmissionId) -> Result<usize, Error> {
        let r = self.admissions()?;
        if subject == 0 || id.number == 0 {
            return Err(Error::Invalid);
        }
        if id.lineage != r.lineage {
            return Err(Error::Lineage);
        }
        r.records
            .iter()
            .position(|r| {
                r.is_some_and(|r| {
                    r.subject == subject && r.admission.is_some_and(|a| a.number == id.number)
                })
            })
            .ok_or(Error::OutcomeUnknown)
    }

    /// Requires fresh caller authorization outside the storage crate. Lookup
    /// neither writes nor resumes execution, including after a remount.
    pub fn admission_by_id(&self, subject: u64, id: AdmissionId) -> Result<Admission<'_>, Error> {
        let slot = self.admission_slot(subject, id)?;
        self.admissions()?.records[slot]
            .as_ref()
            .unwrap()
            .admission_view()
    }

    pub fn admission_by_retry(
        &self,
        subject: u64,
        workspace: u32,
        retry: Retry,
    ) -> Result<Admission<'_>, Error> {
        let r = self.admissions()?;
        if subject == 0 || workspace == 0 || retry.key == 0 || retry.epoch == 0 {
            return Err(Error::Invalid);
        }
        if retry.lineage != r.lineage {
            return Err(Error::Lineage);
        }
        if let Some(record) = r.records.iter().flatten().find(|r| {
            r.subject == subject
                && r.namespace.is_some_and(|(w, _)| w == workspace)
                && r.receipt.retry == retry
        }) {
            return record.admission_view();
        }
        if retry.epoch != r.epoch {
            return Err(Error::ExpiredEpoch);
        }
        Err(Error::OutcomeUnknown)
    }
}
