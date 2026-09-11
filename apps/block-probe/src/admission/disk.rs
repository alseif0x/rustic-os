// SPDX-License-Identifier: Apache-2.0
//! Real native I/O, retaining a copied command while service control runs.
use crate::volume_disk::Disk;
use core::{cell::Cell, task::Poll};
use rustic_fs::{Error, PollDisk, SECTORS};
use rustic_sdk::block::{Error as BlockError, Operation, Status};

pub(super) struct Owned<'a, 'd> {
    disk: &'a mut Disk<'d>,
    count: &'a Cell<usize>,
    pending: Option<(u64, u64, [u8; 512])>,
    held: u8,
}
impl<'a, 'd> Owned<'a, 'd> {
    pub(super) fn new(disk: &'a mut Disk<'d>, count: &'a Cell<usize>) -> Self {
        Self {
            disk,
            count,
            pending: None,
            held: 0,
        }
    }
    fn command(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        let operation = if sector == u64::MAX {
            Operation::Flush
        } else {
            Operation::Write
        };
        if let Some((id, expected_sector, expected_bytes)) = self.pending {
            assert_eq!((sector, *bytes), (expected_sector, expected_bytes));
            // Deliberately retain completion observation across two control polls.
            // This fixture does not simulate a failed or physically delayed write.
            if self.held < 2 {
                self.held += 1;
                return Poll::Pending;
            }
            match self.disk.device.result() {
                Err(BlockError::WouldBlock) => Poll::Pending,
                Ok(done) => {
                    assert_eq!(done.id, id);
                    assert_eq!(done.operation, operation);
                    assert_eq!(done.status, Status::Success);
                    self.pending = None;
                    self.disk.writes += 1;
                    Poll::Ready(Ok(()))
                }
                Err(_) => panic!("native admission completion failed"),
            }
        } else {
            let id = if sector == u64::MAX {
                self.disk.device.flush().unwrap()
            } else {
                assert!(sector < SECTORS);
                self.disk
                    .device
                    .write(self.disk.base + sector, bytes)
                    .unwrap()
            };
            self.pending = Some((id, sector, *bytes));
            self.held = 0;
            self.count.set(self.count.get() + 1);
            Poll::Pending
        }
    }
}
impl PollDisk for Owned<'_, '_> {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        self.command(sector, bytes)
    }
    fn poll_flush(&mut self) -> Poll<Result<(), Error>> {
        self.command(u64::MAX, &[0; 512])
    }
}
