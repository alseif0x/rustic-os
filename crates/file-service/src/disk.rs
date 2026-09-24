// SPDX-License-Identifier: Apache-2.0
//! Synchronous disk adapter for portable dispatch; native dispatch uses real polling.
//!
//! Every command settles inside the call, so a publication driven through it
//! never observes `Pending`. The V7 service uses the same adapter to drive its
//! pollable admission publications to completion inside one request.
use core::task::Poll;
use rustic_fs::{Disk, PollDisk, PollDisk7};
pub(crate) struct Synchronous<'a, D>(pub(crate) &'a mut D);
impl<D: Disk> PollDisk for Synchronous<'_, D> {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), rustic_fs::Error>> {
        Poll::Ready(self.0.write(sector, bytes))
    }
    fn poll_flush(&mut self) -> Poll<Result<(), rustic_fs::Error>> {
        Poll::Ready(self.0.flush())
    }
}
impl<D: Disk> PollDisk7 for Synchronous<'_, D> {
    fn poll_read(
        &mut self,
        sector: u64,
        bytes: &mut [u8; 512],
    ) -> Poll<Result<(), rustic_fs::Error>> {
        Poll::Ready(self.0.read(sector, bytes))
    }
}
