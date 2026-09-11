// SPDX-License-Identifier: Apache-2.0
use super::{AdmissionState, AdmissionStatus, Stored};
use crate::{
    Disk, Error, Kind, MAX_FILE, Publication, Receipt, Replacement, Volume, recovery::Record,
};
impl Volume {
    /// Reserves one of the existing two global slots and persists all arguments
    /// before returning an ID. Does not modify the file or retain authority.
    pub fn admit_replace(
        &mut self,
        disk: &mut impl Disk,
        subject: u64,
        instance: u64,
        request: Replacement,
        bytes: &[u8],
    ) -> Result<AdmissionStatus, Error> {
        self.prepare_admission(disk, subject, instance, request, bytes)?
            .run()
    }

    /// Prepare admission without I/O or an acknowledged ID. Poll under current
    /// service authority; cancelling this guard does not admit the operation.
    #[inline(never)]
    pub fn prepare_admission<'a, D>(
        &'a mut self,
        disk: &'a mut D,
        subject: u64,
        instance: u64,
        request: Replacement,
        bytes: &[u8],
    ) -> Result<Publication<'a, D, AdmissionStatus>, Error> {
        if bytes.len() > MAX_FILE {
            return Err(Error::Size);
        }
        match self.admission_by_retry(subject, request.workspace, request.retry) {
            Ok(old) => {
                if old.request.id != request.id
                    || old.request.version != request.version
                    || old.bytes != bytes
                {
                    return Err(Error::IdempotencyConflict);
                }
                let status = old.status;
                return Ok(Publication::replayed(self, disk, status));
            }
            Err(Error::OutcomeUnknown) => (),
            Err(e) => return Err(e),
        }
        self.resolve(request.workspace, request.id)?;
        let node = self.writable(request.id)?;
        if node.kind != Kind::File {
            return Err(Error::IsDirectory);
        }
        if node.version != request.version {
            return Err(Error::Version);
        }
        let slot = self
            .admissions()?
            .records
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Full)?;
        let number = self.sequence().checked_add(1).ok_or(Error::Exhausted)?;
        let instance = if instance == 0 { number } else { instance };
        if instance > number {
            return Err(Error::Invalid);
        }
        let mut record = Record {
            subject,
            receipt: Receipt {
                retry: request.retry,
                id: request.id,
                previous: request.version,
                committed: 0,
                length: bytes.len() as u16,
            },
            namespace: Some((request.workspace, instance)),
            bytes: [0; MAX_FILE],
            admission: Some(Stored {
                number,
                state: AdmissionState::Admitted,
                terminal: 0,
            }),
        };
        record.bytes[..bytes.len()].copy_from_slice(bytes);
        let status = record.admission_view()?.status;
        let mut next = self.metadata.clone();
        next.recovery.as_mut().unwrap().records[slot] = Some(record);
        Publication::metadata(self, disk, next, status)
    }
}
