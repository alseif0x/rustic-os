// SPDX-License-Identifier: Apache-2.0
//! Native block adapter for the testable copied-request poller.
use super::Disk;
use core::task::Poll;
use rustic_file_server::Requests;
use rustic_fs::{Error, PollDisk, PollDisk7};
use rustic_sdk::block::{Completion, Device, Error as BlockError, Operation};

struct NativeRequests<'a>(&'a mut Device);

impl Requests for NativeRequests<'_> {
    fn read(&mut self, sector: u64) -> Result<u64, BlockError> {
        self.0.read(sector)
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<u64, BlockError> {
        self.0.write(sector, bytes)
    }

    fn flush(&mut self) -> Result<u64, BlockError> {
        self.0.flush()
    }

    fn result(&mut self) -> Result<Completion, BlockError> {
        self.0.result()
    }
}

impl Disk {
    fn poll(
        &mut self,
        operation: Operation,
        sector: u64,
        data: &[u8; 512],
    ) -> Poll<Result<(), Error>> {
        let mut device = NativeRequests(&mut self.device);
        self.poller.poll(&mut device, operation, sector, data, None)
    }
}

impl PollDisk for Disk {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        self.poll(Operation::Write, sector, bytes)
    }

    fn poll_flush(&mut self) -> Poll<Result<(), Error>> {
        self.poll(Operation::Flush, 0, &[0; 512])
    }
}

impl PollDisk7 for Disk {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), Error>> {
        let mut device = NativeRequests(&mut self.device);
        self.poller
            .poll(&mut device, Operation::Read, sector, &[0; 512], Some(bytes))
    }
}
