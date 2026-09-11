// SPDX-License-Identifier: Apache-2.0
//! Retain one copied native request across service-control polls; never resubmit it.
use super::Disk;
use core::task::Poll;
use rustic_fs::{Error, PollDisk};
use rustic_sdk::block::{Error as BlockError, Operation, Status};

pub(super) struct Pending {
    id: u64,
    operation: Operation,
    sector: u64,
    data: [u8; 512],
}
impl Disk {
    fn poll(
        &mut self,
        operation: Operation,
        sector: u64,
        data: &[u8; 512],
    ) -> Poll<Result<(), Error>> {
        if self.fenced {
            return Poll::Ready(Err(Error::Uncertain));
        }
        self.fenced = true;
        if let Some(pending) = &self.pending {
            if pending.operation != operation || pending.sector != sector || &pending.data != data {
                return Poll::Ready(Err(Error::Uncertain));
            }
            match self.device.result() {
                Err(BlockError::WouldBlock) => {
                    self.fenced = false;
                    Poll::Pending
                }
                Ok(done)
                    if done.id == pending.id
                        && done.operation == operation
                        && done.status == Status::Success =>
                {
                    self.pending = None;
                    self.fenced = false;
                    Poll::Ready(Ok(()))
                }
                _ => Poll::Ready(Err(Error::Uncertain)),
            }
        } else {
            let id = match operation {
                Operation::Write => self.device.write(sector, data),
                Operation::Flush => self.device.flush(),
                Operation::Read => return Poll::Ready(Err(Error::Invalid)),
            };
            match id {
                Ok(id) => {
                    self.pending = Some(Pending {
                        id,
                        operation,
                        sector,
                        data: *data,
                    });
                    self.fenced = false;
                    Poll::Pending
                }
                Err(_) => Poll::Ready(Err(Error::Uncertain)),
            }
        }
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
