// SPDX-License-Identifier: Apache-2.0
mod support;
use core::task::Poll;
use rustic_fs::{
    Disk, Error, Kind, PollDisk, PublicationCancel as Cancel, PublicationPhase as Phase, Volume,
};
use std::{cell::Cell, rc::Rc};
use support::MemoryDisk;

struct Held {
    disk: MemoryDisk,
    release: Rc<Cell<bool>>,
    admitted: Option<(u64, [u8; 512])>,
    panic: bool,
}
impl PollDisk for Held {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        if let Some(expected) = self.admitted {
            assert_eq!(expected, (sector, *bytes));
            if self.release.get() {
                self.admitted = None;
                Poll::Ready(Ok(()))
            } else {
                Poll::Pending
            }
        } else {
            self.disk.write(sector, bytes).unwrap();
            self.admitted = Some((sector, *bytes));
            assert!(!self.panic, "adapter unwind after real submission");
            Poll::Pending
        }
    }
    fn poll_flush(&mut self) -> Poll<Result<(), Error>> {
        panic!("no flush is allowed after cancellation of the first write")
    }
}
fn base() -> (Held, Volume, rustic_fs::Node) {
    let mut disk = MemoryDisk::new();
    let mut volume = Volume::initialize(&mut disk).unwrap();
    let file = volume.create(&mut disk, 4, b"file", Kind::File).unwrap();
    (
        Held {
            disk,
            release: Rc::default(),
            admitted: None,
            panic: false,
        },
        volume,
        file,
    )
}

#[test]
fn cancellation_is_only_confirmed_after_the_admitted_command_settles() {
    let (mut disk, mut volume, file) = base();
    let release = disk.release.clone();
    let start = disk.disk.operations;
    let mut write = volume
        .prepare_replace(&mut disk, file.id, file.version, b"after")
        .unwrap();
    assert_eq!(write.poll_advance(), Poll::Pending);
    for _ in 0..4 {
        assert!(write.pending());
        assert_eq!(write.cancel(), Ok(Cancel::Draining));
        assert!(write.result().is_none());
        assert_eq!(write.poll_advance(), Poll::Pending);
    }
    release.set(true);
    assert_eq!(write.poll_advance(), Poll::Ready(Ok(Phase::Cancelled)));
    assert!(!write.pending());
    assert_eq!(write.cancel(), Ok(Cancel::Cancelled));
    assert_eq!(write.poll_advance(), Poll::Ready(Ok(Phase::Cancelled)));
    drop(write);
    assert_eq!(disk.disk.operations, start + 1);
    assert_eq!(volume.stat(file.id).unwrap().version, file.version);
    for durable in [false, true] {
        let mut recovered = disk.disk.recover(durable);
        assert_eq!(
            Volume::mount(&mut recovered)
                .unwrap()
                .stat(file.id)
                .unwrap()
                .length,
            0
        );
    }
}

#[test]
fn drop_pending_forget_and_adapter_unwind_keep_the_volume_fenced() {
    for mode in 0..3 {
        let (mut disk, mut volume, file) = base();
        disk.panic = mode == 2;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut write = volume
                .prepare_replace(&mut disk, file.id, file.version, b"after")
                .unwrap();
            assert_eq!(write.poll_advance(), Poll::Pending);
            assert_eq!(write.cancel(), Ok(Cancel::Draining));
            if mode == 1 {
                core::mem::forget(write);
            }
        }));
        assert_eq!(result.is_err(), mode == 2);
        assert!(disk.admitted.is_some());
        assert!(matches!(volume.stat(file.id), Err(Error::Uncertain)));
        assert!(matches!(
            volume.prepare_replace(&mut disk, file.id, file.version, b"unsafe"),
            Err(Error::Uncertain)
        ));
    }
}
