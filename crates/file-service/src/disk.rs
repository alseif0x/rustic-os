// SPDX-License-Identifier: Apache-2.0
//! Synchronous disk adapter for portable dispatch; native dispatch uses real polling.
use core::task::Poll;
use rustic_fs::{Disk, PollDisk};
pub(crate) struct Synchronous<'a, D>(pub(crate) &'a mut D);
impl<D: Disk> PollDisk for Synchronous<'_, D> {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), rustic_fs::Error>> {
        Poll::Ready(self.0.write(sector, bytes))
    }
    fn poll_flush(&mut self) -> Poll<Result<(), rustic_fs::Error>> {
        Poll::Ready(self.0.flush())
    }
}
