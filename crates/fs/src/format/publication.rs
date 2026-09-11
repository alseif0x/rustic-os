// SPDX-License-Identifier: Apache-2.0
//! The single flush-ordered metadata publication path, shared by all mutations.
use super::Metadata;
use crate::{Disk, Error, checksum::crc, publication::Command, storage::header};

impl Metadata {
    pub(crate) fn write_steps(&self) -> usize {
        if self.recovery.is_some() { 14 } else { 7 }
    }

    /// Each successful step settles exactly one write or flush. The caller must
    /// run steps in order on the same exclusively owned disk, without overlap.
    pub(crate) fn write_step(
        &self,
        disk: &mut impl Disk,
        bank: u8,
        step: usize,
    ) -> Result<(), Error> {
        self.command(bank, step)?.execute(disk)
    }

    pub(crate) fn command(&self, bank: u8, step: usize) -> Result<Command, Error> {
        let records = if self.recovery.is_some() { 7 } else { 0 };
        if step < records {
            let bytes = self.recovery.as_ref().unwrap().encode();
            return Ok(Command::Write(
                160 + u64::from(bank) * 7 + step as u64,
                bytes.as_chunks::<512>().0[step],
            ));
        }
        match step - records {
            index @ 0..=3 => Ok(Command::Write(
                header(bank) + 1 + index as u64,
                self.bytes().as_chunks::<512>().0[index],
            )),
            4 | 6 => Ok(Command::Flush),
            5 => Ok(Command::Write(header(bank), self.publication_header())),
            _ => Err(Error::Invalid),
        }
    }

    pub(crate) fn write(&self, disk: &mut impl Disk, bank: u8) -> Result<(), Error> {
        for step in 0..self.write_steps() {
            self.write_step(disk, bank, step)?;
        }
        Ok(())
    }

    fn publication_header(&self) -> [u8; 512] {
        let mut bytes = [0; 512];
        bytes[..8].copy_from_slice(b"RUSTFS1\0");
        let version = self
            .recovery
            .as_ref()
            .map_or(1u16, |r| if r.scoped { 3 } else { 2 });
        bytes[8..10].copy_from_slice(&version.to_le_bytes());
        bytes[10..12].copy_from_slice(&512u16.to_le_bytes());
        bytes[12..20].copy_from_slice(&self.sequence.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.next.to_le_bytes());
        bytes[24..28].copy_from_slice(&crc(&self.bytes()).to_le_bytes());
        let recovery_crc = self.recovery.as_ref().map_or(0, |r| crc(&r.encode()));
        bytes[32..36].copy_from_slice(&recovery_crc.to_le_bytes());
        let hash = crc(&bytes);
        bytes[28..32].copy_from_slice(&hash.to_le_bytes());
        bytes
    }
}
