// SPDX-License-Identifier: Apache-2.0
use super::{content, control, disk::Owned, open, request};
use crate::volume_disk::Disk;
use core::cell::Cell;
use rustic_file_service::Server;
use rustic_fs::{AdmissionState as State, Error};

#[inline(never)]
pub(super) fn verify(disk: &mut Disk<'_>, phase: u64) {
    let mut s = Server::new(open(disk, phase));
    let request = request(&s.volume, 53);
    if phase == 1 {
        control::admit(&mut s, disk, request, b"unexecuted");
    }
    check(&mut s, disk, request);
}

// Keep replay/publication buffers outside the mount/setup stack frame.
#[inline(never)]
fn check(s: &mut Server, disk: &mut Disk<'_>, request: rustic_fs::Replacement) {
    let start = disk.writes;
    let a = s.volume.admission_by_retry(9, 4, request.retry).unwrap();
    assert_eq!(a.status.state, State::Admitted);
    assert_eq!(a.receipt, None);
    assert_eq!(a.bytes, b"unexecuted");
    let status = a.status;
    let caller = control::grant(s, false);
    assert_eq!(s.admission_status(caller, status.id, 1).unwrap(), status);
    let count = Cell::new(0);
    assert_eq!(
        s.admit_with(
            &mut Owned::new(disk, &count),
            caller,
            request,
            b"unexecuted",
            |_, _| 1
        )
        .unwrap(),
        status
    );
    assert_eq!(
        s.execute_admission_with(&mut Owned::new(disk, &count), caller, status.id, |_, _| 1),
        Err(rustic_sdk::abi::files::Error::Denied)
    );
    assert_eq!(count.get(), 0);
    let v = &mut s.volume;
    assert_eq!(v.admission_by_id(9, status.id).unwrap().status, status);
    assert_eq!(
        v.admit_replace(disk, 9, 0, request, b"unexecuted").unwrap(),
        status
    );
    assert_eq!(v.advance_epoch(disk), Err(Error::Busy));
    assert_eq!(v.stat(request.id).unwrap().version, request.version);
    content(v, disk, b"before");
    assert_eq!(disk.writes, start);
}
