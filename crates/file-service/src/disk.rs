// SPDX-License-Identifier: Apache-2.0
//! Synchronous disk adapter for portable dispatch; native dispatch uses real polling.
//!
//! Every command settles inside the call, so a publication driven through it
//! never observes `Pending`. [`crate::Server7::handle`] serves requests through
//! this adapter, so its admission publications settle inside one request with
//! no owner control between their polls; the native service drives them over
//! its pollable disk instead ([`crate::Server7::handle_with`]).
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
/// Blocking commands pass straight through, so one adapter serves both the
/// blocking and the pollable paths of a request.
impl<D: Disk> Disk for Synchronous<'_, D> {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), rustic_fs::Error> {
        self.0.read(sector, bytes)
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), rustic_fs::Error> {
        self.0.write(sector, bytes)
    }
    fn flush(&mut self) -> Result<(), rustic_fs::Error> {
        self.0.flush()
    }
}
