// SPDX-License-Identifier: Apache-2.0
//! Exclusive copied-sector view of one disjoint V7 fixture volume.
use crate::persistence::finish;
use core::task::Poll;
use rustic_fs::{Error, PollDisk, PollDisk7, Volume7};
use rustic_sdk::block::{Device, Operation};

static mut VOLUME7: Volume7 = Volume7::EMPTY;

pub(super) struct Disk<'a> {
    pub(super) device: &'a Device,
    pub(super) writes: usize,
    pub(super) events: usize,
    pub(super) base: u64,
    pub(super) sectors: u64,
}

impl<'a> Disk<'a> {
    pub(super) fn at(device: &'a Device, base: u64, sectors: u64) -> Self {
        assert_eq!(sectors, rustic_fs::format7::VOLUME_SECTORS);
        Self {
            device,
            writes: 0,
            events: 0,
            base,
            sectors,
        }
    }

    fn absolute(&self, sector: u64) -> u64 {
        assert!(sector < self.sectors);
        self.base + sector
    }
}

impl rustic_fs::Disk for Disk<'_> {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        let result = finish(
            self.device,
            self.device.read(self.absolute(sector)).unwrap(),
        );
        assert_eq!(result.operation, Operation::Read);
        *bytes = result.data;
        self.events += 1;
        Ok(())
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        let result = finish(
            self.device,
            self.device.write(self.absolute(sector), bytes).unwrap(),
        );
        assert_eq!(result.operation, Operation::Write);
        self.writes += 1;
        self.events += 1;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Error> {
        let result = finish(self.device, self.device.flush().unwrap());
        assert_eq!(result.operation, Operation::Flush);
        self.writes += 1;
        self.events += 1;
        Ok(())
    }
}

impl PollDisk for Disk<'_> {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        Poll::Ready(rustic_fs::Disk::write(self, sector, bytes))
    }

    fn poll_flush(&mut self) -> Poll<Result<(), Error>> {
        Poll::Ready(rustic_fs::Disk::flush(self))
    }
}

impl PollDisk7 for Disk<'_> {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), Error>> {
        Poll::Ready(rustic_fs::Disk::read(self, sector, bytes))
    }
}

#[inline(never)]
pub(super) fn mount(disk: &mut Disk<'_>) -> &'static mut Volume7 {
    // SAFETY: one native fixture role mounts once in this single-threaded
    // process; no other code names VOLUME7 or outlives that role's exclusive use.
    let volume = unsafe { &mut *core::ptr::addr_of_mut!(VOLUME7) };
    volume.mount_into(disk).unwrap();
    volume
}
