// SPDX-License-Identifier: Apache-2.0
//! Model an admitted command whose completion is withheld independently of control.
use rustic_fs::{Disk, Error, PollDisk};
use std::{cell::Cell, rc::Rc, task::Poll};
#[derive(Default)]
pub struct Signals {
    pub submitted: Cell<usize>,
    pub settled: Cell<usize>,
    pub release: Cell<bool>,
    pub fail: Cell<bool>,
}
pub struct Deferred<D> {
    pub disk: D,
    pub signals: Rc<Signals>,
    pending: Option<(u64, [u8; 512])>,
}
impl<D> Deferred<D> {
    pub fn new(disk: D) -> Self {
        Self {
            disk,
            signals: Rc::default(),
            pending: None,
        }
    }
}
impl<D: Disk> Deferred<D> {
    fn command(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        if let Some(expected) = &self.pending {
            assert_eq!(
                expected,
                &(sector, *bytes),
                "a pending command changed or was resubmitted"
            );
            if !self.signals.release.get() {
                return Poll::Pending;
            }
            self.pending = None;
            self.signals.settled.set(self.signals.settled.get() + 1);
            Poll::Ready(if self.signals.fail.get() {
                Err(Error::Io)
            } else {
                Ok(())
            })
        } else {
            self.pending = Some((sector, *bytes));
            self.signals.submitted.set(self.signals.submitted.get() + 1);
            if sector == u64::MAX {
                self.disk.flush().unwrap();
            } else {
                self.disk.write(sector, bytes).unwrap();
            }
            Poll::Pending
        }
    }
}
impl<D: Disk> PollDisk for Deferred<D> {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        self.command(sector, bytes)
    }
    fn poll_flush(&mut self) -> Poll<Result<(), Error>> {
        self.command(u64::MAX, &[0; 512])
    }
}
