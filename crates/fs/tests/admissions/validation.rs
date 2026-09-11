// SPDX-License-Identifier: Apache-2.0
use crate::base;
use rustic_fs::{Disk, Error, Volume};

#[test]
fn adapter_unwind_fences_admission_and_terminal_metadata_transitions() {
    struct Panics;
    impl Disk for Panics {
        fn read(&mut self, _: u64, _: &mut [u8; 512]) -> Result<(), Error> {
            panic!("unexpected read")
        }
        fn write(&mut self, _: u64, _: &[u8; 512]) -> Result<(), Error> {
            panic!("adapter unwind")
        }
        fn flush(&mut self) -> Result<(), Error> {
            panic!("unexpected flush")
        }
    }
    let (mut disk, request) = base();
    let mut v = Volume::mount(&mut disk).unwrap();
    v.enable_admissions(&mut disk).unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.admit_replace(
            &mut Panics,
            9,
            0,
            request,
            b"after"
        )))
        .is_err()
    );
    assert!(matches!(v.stat(request.id), Err(Error::Uncertain)));
    let mut v = Volume::mount(&mut disk).unwrap();
    let id = v
        .admit_replace(&mut disk, 9, 0, request, b"after")
        .unwrap()
        .id;
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.cancel_admission(
            &mut Panics,
            9,
            id
        )))
        .is_err()
    );
    assert!(matches!(v.stat(request.id), Err(Error::Uncertain)));
}

#[test]
fn forgotten_or_late_abandoned_execution_cannot_publish_a_false_cancellation() {
    let (mut disk, request) = base();
    let mut v = Volume::mount(&mut disk).unwrap();
    v.enable_admissions(&mut disk).unwrap();
    let id = v
        .admit_replace(&mut disk, 9, 0, request, b"after")
        .unwrap()
        .id;
    std::mem::forget(v.prepare_admitted(&mut disk, 9, id).unwrap());
    assert_eq!(v.cancel_admission(&mut disk, 9, id), Err(Error::Uncertain));
    let mut v = Volume::mount(&mut disk).unwrap();
    let mut write = v.prepare_admitted(&mut disk, 9, id).unwrap();
    for _ in 0..16 {
        write.advance().unwrap();
    }
    drop(write);
    assert_eq!(v.cancel_admission(&mut disk, 9, id), Err(Error::Uncertain));
}
