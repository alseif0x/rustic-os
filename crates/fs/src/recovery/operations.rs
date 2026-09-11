// SPDX-License-Identifier: Apache-2.0
use super::{Receipt, Record, Recovery, Retry};
use crate::{Disk, Error, MAX_FILE, Volume};
impl Volume {
    /// One-way metadata upgrade, authorized by the caller. Existing file extents are untouched.
    pub fn enable_recovery(
        &mut self,
        disk: &mut impl Disk,
        lineage: [u8; 16],
    ) -> Result<(), Error> {
        self.ready()?;
        if let Some(recovery) = &self.metadata.recovery {
            return if recovery.lineage == lineage {
                Ok(())
            } else {
                Err(Error::Lineage)
            };
        }
        let initial = Recovery::new(lineage)?;
        let expected = initial.encode();
        // Only zeros or a torn initial image with this lineage may be resumed.
        // An old v1 header has never published a receipt in these scratch sectors.
        for sector in 160..crate::SECTORS {
            let mut bytes = [0; 512];
            disk.read(sector, &mut bytes)?;
            let offset = ((sector - 160) as usize % 7) * 512;
            if bytes
                .iter()
                .zip(&expected[offset..offset + 512])
                .any(|(a, b)| *a != 0 && a != b)
            {
                return Err(Error::Corrupt);
            }
        }
        let mut next = self.metadata.clone();
        next.recovery = Some(initial);
        self.commit(disk, next)
    }
    pub fn recovery_info(&self) -> Result<([u8; 16], u64), Error> {
        self.ready()?;
        let r = self.metadata.recovery.as_ref().ok_or(Error::Unsupported)?;
        Ok((r.lineage, r.epoch))
    }
    pub fn receipt(&self, subject: u64, retry: Retry) -> Result<Receipt, Error> {
        self.ready()?;
        self.metadata
            .recovery
            .as_ref()
            .ok_or(Error::Unsupported)?
            .find(subject, retry)?
            .map(|r| r.receipt)
            .ok_or(Error::OutcomeUnknown)
    }
    /// Evidence and content/version share the same checksummed publication header.
    /// No queued/running operation is acknowledged: staging has no durable acceptance.
    pub fn replace_tracked(
        &mut self,
        disk: &mut impl Disk,
        subject: u64,
        retry: Retry,
        id: u32,
        version: u64,
        bytes: &[u8],
    ) -> Result<Receipt, Error> {
        self.ready()?;
        if bytes.len() > MAX_FILE {
            return Err(Error::Size);
        }
        let recovery = self.metadata.recovery.as_ref().ok_or(Error::Unsupported)?;
        if let Some(r) = recovery.find(subject, retry)? {
            if r.receipt.id != id
                || r.receipt.previous != version
                || r.receipt.length as usize != bytes.len()
                || &r.bytes[..bytes.len()] != bytes
            {
                return Err(Error::IdempotencyConflict);
            }
            return Ok(r.receipt);
        }
        let slot = recovery
            .records
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Full)?;
        let receipt = Receipt {
            retry,
            id,
            previous: version,
            committed: self
                .metadata
                .sequence
                .checked_add(1)
                .ok_or(Error::Exhausted)?,
            length: bytes.len() as u16,
        };
        let mut record = Record {
            subject,
            receipt,
            namespace: None,
            bytes: [0; MAX_FILE],
            admission: None,
        };
        record.bytes[..bytes.len()].copy_from_slice(bytes);
        let mut next = self.metadata.clone();
        next.recovery.as_mut().unwrap().records[slot] = Some(record);
        self.replace_recorded(disk, id, version, bytes, next)?;
        Ok(receipt)
    }
    /// Owner-controlled count retention; epoch and removal publish together.
    /// A torn transition leaves either the old records/epoch or the new fenced epoch.
    pub fn advance_epoch(&mut self, disk: &mut impl Disk) -> Result<u64, Error> {
        self.ready()?;
        let mut next = self.metadata.clone();
        let r = next.recovery.as_mut().ok_or(Error::Unsupported)?;
        if r.records.iter().flatten().any(|r| {
            r.admission
                .is_some_and(|a| a.state == crate::AdmissionState::Admitted)
        }) {
            return Err(Error::Busy);
        }
        r.epoch = r.epoch.checked_add(1).ok_or(Error::Exhausted)?;
        r.records.fill(None);
        let epoch = r.epoch;
        self.commit(disk, next)?;
        Ok(epoch)
    }
}
