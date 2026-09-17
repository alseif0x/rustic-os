// SPDX-License-Identifier: Apache-2.0
//! A sector-addressed host file, so a volume image is exercised as an image.
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use rustic_fs::{Disk, Error};

/// A host file that is exactly `sectors` long. Holes read as zeros and reads
/// outside the declared length are an error rather than silent zeros, so a
/// truncated image cannot be mistaken for a valid volume.
pub(crate) struct FileDisk {
    file: File,
    sectors: u64,
}

impl FileDisk {
    pub(crate) fn create(path: &Path, sectors: u64) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
        file.set_len(sectors * 512)
            .map_err(|error| format!("cannot size {}: {error}", path.display()))?;
        Ok(Self { file, sectors })
    }

    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
        let sectors = file
            .metadata()
            .map_err(|error| format!("cannot size {}: {error}", path.display()))?
            .len()
            / 512;
        Ok(Self { file, sectors })
    }

    pub(crate) fn sectors(&self) -> u64 {
        self.sectors
    }
}

impl Disk for FileDisk {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        if sector >= self.sectors {
            return Err(Error::Io);
        }
        self.file
            .seek(SeekFrom::Start(sector * 512))
            .map_err(|_| Error::Io)?;
        self.file.read_exact(bytes).map_err(|_| Error::Io)
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        if sector >= self.sectors {
            return Err(Error::Io);
        }
        self.file
            .seek(SeekFrom::Start(sector * 512))
            .map_err(|_| Error::Io)?;
        self.file.write_all(bytes).map_err(|_| Error::Io)
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.file.sync_all().map_err(|_| Error::Io)
    }
}
