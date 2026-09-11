// SPDX-License-Identifier: Apache-2.0
use super::super::*;
use rustic_fs::{Disk, PollDisk};
use std::{cell::Cell, rc::Rc, task::Poll};

struct Held {
    disk: MemoryDisk,
    release: Rc<Cell<bool>>,
    pending: Option<(u64, [u8; 512])>,
    unwind: bool,
}
impl Held {
    fn command(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        if let Some(expected) = self.pending {
            assert_eq!(expected, (sector, *bytes));
            if !self.release.get() {
                return Poll::Pending;
            }
            self.pending = None;
            Poll::Ready(Ok(()))
        } else {
            if sector == u64::MAX {
                self.disk.flush().unwrap();
            } else {
                self.disk.write(sector, bytes).unwrap();
            }
            self.pending = Some((sector, *bytes));
            assert!(!self.unwind, "adapter unwound after submission");
            Poll::Pending
        }
    }
}
impl PollDisk for Held {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        self.command(sector, bytes)
    }
    fn poll_flush(&mut self) -> Poll<Result<(), Error>> {
        self.command(u64::MAX, &[0; 512])
    }
}

#[test]
fn metadata_guards_drain_each_pending_command_and_never_confuse_abandonment_with_terminal_state() {
    for terminal in [false, true] {
        for cut in 1..=14 {
            let (mut disk, request) = base();
            let mut v = Volume::mount(&mut disk).unwrap();
            v.enable_admissions(&mut disk).unwrap();
            let accepted = if terminal {
                Some(v.admit_replace(&mut disk, 9, 0, request, b"after").unwrap())
            } else {
                None
            };
            let start = disk.operations;
            let mut disk = Held {
                disk,
                release: Rc::new(Cell::new(true)),
                pending: None,
                unwind: false,
            };
            let release = disk.release.clone();
            let mut write = if let Some(a) = accepted {
                v.prepare_cancellation(&mut disk, 9, a.id).unwrap()
            } else {
                v.prepare_admission(&mut disk, 9, 0, request, b"after")
                    .unwrap()
            };
            assert_eq!(write.result(), None);
            for _ in 1..cut {
                assert_eq!(write.poll_advance(), Poll::Pending);
                assert!(matches!(write.poll_advance(), Poll::Ready(Ok(_))));
            }
            release.set(false);
            assert_eq!(write.poll_advance(), Poll::Pending);
            for _ in 0..3 {
                assert_eq!(
                    write.cancel(),
                    Ok(if cut >= 13 {
                        Cancel::TooLate
                    } else {
                        Cancel::Draining
                    })
                );
                assert_eq!(write.result(), None);
                assert_eq!(write.poll_advance(), Poll::Pending);
            }
            release.set(true);
            while !matches!(write.phase(), Phase::Cancelled | Phase::Committed) {
                if let Poll::Ready(result) = write.poll_advance() {
                    result.unwrap();
                }
            }
            assert_eq!(write.result().is_some(), cut >= 13);
            drop(write);
            assert_eq!(
                disk.disk.operations - start,
                if cut >= 13 { 14 } else { cut }
            );
            // Neither metadata transition may touch any file data extent.
            for durable in [false, true] {
                let mut recovered = disk.disk.recover(durable);
                let v = Volume::mount(&mut recovered).unwrap();
                content(&v, &mut recovered, request.id, b"before");
                assert_eq!(v.stat(request.id).unwrap().version, request.version);
                let old = v.admission_by_retry(9, 4, request.retry);
                if !terminal && cut < 13 {
                    assert!(matches!(old, Err(Error::OutcomeUnknown)));
                } else {
                    assert_eq!(
                        old.unwrap().status.state,
                        if terminal && cut >= 13 {
                            State::Cancelled
                        } else {
                            State::Admitted
                        }
                    );
                }
            }
        }
    }
}

#[test]
fn forgotten_pending_and_unwound_metadata_guards_fence_both_admission_and_cancellation() {
    for terminal in [false, true] {
        for mode in 0..3 {
            let (mut disk, request) = base();
            let mut v = Volume::mount(&mut disk).unwrap();
            v.enable_admissions(&mut disk).unwrap();
            let accepted = if terminal {
                Some(v.admit_replace(&mut disk, 9, 0, request, b"after").unwrap())
            } else {
                None
            };
            let mut disk = Held {
                disk,
                release: Rc::default(),
                pending: None,
                unwind: mode == 2,
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut write = if let Some(a) = accepted {
                    v.prepare_cancellation(&mut disk, 9, a.id).unwrap()
                } else {
                    v.prepare_admission(&mut disk, 9, 0, request, b"after")
                        .unwrap()
                };
                assert_eq!(write.poll_advance(), Poll::Pending);
                if mode == 1 {
                    core::mem::forget(write);
                }
            }));
            assert_eq!(result.is_err(), mode == 2);
            assert!(matches!(v.stat(request.id), Err(Error::Uncertain)));
            assert!(disk.pending.is_some());
        }
    }
}
