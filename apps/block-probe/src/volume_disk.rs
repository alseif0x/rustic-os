// SPDX-License-Identifier: Apache-2.0
//! Exclusive copied-sector view of the fixture's disjoint scratch volume.
use crate::persistence::finish;
use rustic_fs::{Error, SECTORS};
use rustic_sdk::block::{Device, Operation};

pub(super) struct Disk<'a> {
    pub(super) device: &'a Device,
    pub(super) writes: usize,
    pub(super) base: u64,
}
impl rustic_fs::Disk for Disk<'_> {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        assert!(sector < SECTORS);
        let result = finish(self.device, self.device.read(self.base + sector).unwrap());
        assert_eq!(result.operation, Operation::Read);
        *bytes = result.data;
        Ok(())
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        assert!(sector < SECTORS);
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
