// SPDX-License-Identifier: Apache-2.0
//! Exclusive copied-sector view of the fixture's disjoint scratch volume.
use crate::persistence::finish;
use rustic_fs::Error;
use rustic_sdk::block::{Device, Operation};

/// A sector-bounded view of the fixture's disk: `base` is the first sector of
/// the volume and `sectors` how long that volume is, so the v5 volumes and the
/// v6 workspace cannot address each other.
pub(super) struct Disk<'a> {
    pub(super) device: &'a Device,
    pub(super) writes: usize,
    pub(super) base: u64,
    pub(super) sectors: u64,
}

impl<'a> Disk<'a> {
    pub(super) fn at(device: &'a Device, base: u64, sectors: u64) -> Self {
        Self {
            device,
            writes: 0,
            base,
            sectors,
        }
    }
}
impl rustic_fs::Disk for Disk<'_> {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        assert!(sector < self.sectors);
        let result = finish(self.device, self.device.read(self.base + sector).unwrap());
        assert_eq!(result.operation, Operation::Read);
        *bytes = result.data;
        Ok(())
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        assert!(sector < self.sectors);
        let result = finish(
            self.device,
            self.device.write(self.base + sector, bytes).unwrap(),
        );
        assert_eq!(result.operation, Operation::Write);
        self.writes += 1;
        Ok(())
    }
    fn flush(&mut self) -> Result<(), Error> {
        let result = finish(self.device, self.device.flush().unwrap());
        assert_eq!(result.operation, Operation::Flush);
        self.writes += 1;
        Ok(())
    }
}
