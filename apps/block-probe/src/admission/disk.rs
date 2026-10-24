// SPDX-License-Identifier: Apache-2.0
//! Real native V7 I/O, retaining each copied command while service control runs.
use crate::volume_disk::Disk;
use core::{cell::Cell, task::Poll};
use rustic_fs::{Error, PollDisk, PollDisk7};
use rustic_sdk::block::{Error as BlockError, Operation, Status};

enum Command {
    Read(u64),
    Write(u64),
    Flush,
}

pub(super) struct Owned<'a, 'd> {
    disk: &'a mut Disk<'d>,
    count: &'a Cell<usize>,
    pending: Option<(u64, Command)>,
    read_data: [u8; 512],
    write_data: [u8; 512],
    held: u8,
}

impl<'a, 'd> Owned<'a, 'd> {
    pub(super) fn new(disk: &'a mut Disk<'d>, count: &'a Cell<usize>) -> Self {
        Self {
            disk,
            count,
            pending: None,
            read_data: [0; 512],
            write_data: [0; 512],
            held: 0,
        }
    }

    fn submit(&mut self, command: Command) -> Poll<Result<(), Error>> {
        if let Some((id, pending)) = self.pending.take() {
            if self.held < 2 {
                self.held += 1;
                self.pending = Some((id, pending));
                return Poll::Pending;
            }
            let operation = match pending {
                Command::Read(_) => Operation::Read,
                Command::Write(_) => Operation::Write,
                Command::Flush => Operation::Flush,
            };
            match self.disk.device.result() {
                Err(BlockError::WouldBlock) => {
                    self.pending = Some((id, pending));
                    Poll::Pending
                }
                Ok(done) => {
                    assert_eq!(done.id, id);
                    assert_eq!(done.operation, operation);
                    assert_eq!(done.status, Status::Success);
                    if matches!(pending, Command::Read(_)) {
                        self.read_data = done.data;
                    } else {
                        self.disk.writes += 1;
                    }
                    self.disk.events += 1;
                    Poll::Ready(Ok(()))
                }
                Err(_) => panic!("native admission completion failed"),
            }
        } else {
            let id = match &command {
                Command::Read(sector) => self.disk.device.read(self.absolute(*sector)).unwrap(),
                Command::Write(sector) => self
                    .disk
                    .device
                    .write(self.absolute(*sector), &self.write_data)
                    .unwrap(),
                Command::Flush => self.disk.device.flush().unwrap(),
            };
            self.pending = Some((id, command));
            self.held = 0;
            self.count.set(self.count.get() + 1);
            Poll::Pending
        }
    }

    fn absolute(&self, sector: u64) -> u64 {
        assert!(sector < self.disk.sectors);
        self.disk.base + sector
    }
}

impl rustic_fs::Disk for Owned<'_, '_> {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        assert!(self.pending.is_none());
        rustic_fs::Disk::read(self.disk, sector, bytes)
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        assert!(self.pending.is_none());
        rustic_fs::Disk::write(self.disk, sector, bytes)
    }

    fn flush(&mut self) -> Result<(), Error> {
        assert!(self.pending.is_none());
        rustic_fs::Disk::flush(self.disk)
    }
}

impl PollDisk for Owned<'_, '_> {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        self.write_data = *bytes;
        self.submit(Command::Write(sector))
    }

    fn poll_flush(&mut self) -> Poll<Result<(), Error>> {
        self.submit(Command::Flush)
    }
}

impl PollDisk7 for Owned<'_, '_> {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), Error>> {
        if let Some((_, Command::Read(pending))) = &self.pending {
            assert_eq!(*pending, sector);
        } else if self.pending.is_some() {
            panic!("read replaced another pending command");
        }
        match self.submit(Command::Read(sector)) {
            Poll::Ready(Ok(())) => {
                *bytes = self.read_data;
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}
