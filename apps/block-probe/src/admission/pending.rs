// SPDX-License-Identifier: Apache-2.0
use super::{content, open, request};
use crate::volume_disk::Disk;
use rustic_fs::{AdmissionState as State, Error};

#[inline(never)]
pub(super) fn verify(disk: &mut Disk<'_>, phase: u64) {
    let mut v = open(disk, phase);
    let request = request(&v, 53);
    if phase == 1 {
        v.admit_replace(disk, 9, 0, request, b"unexecuted").unwrap();
    }
    let start = disk.writes;
    let a = v.admission_by_retry(9, 4, request.retry).unwrap();
    assert_eq!(a.status.state, State::Admitted);
    assert_eq!(a.receipt, None);
    assert_eq!(a.bytes, b"unexecuted");
    let status = a.status;
    assert_eq!(v.admission_by_id(9, status.id).unwrap().status, status);
    assert_eq!(
        v.admit_replace(disk, 9, 0, request, b"unexecuted").unwrap(),
        status
    );
    assert_eq!(v.advance_epoch(disk), Err(Error::Busy));
    assert_eq!(v.stat(request.id).unwrap().version, request.version);
    content(&v, disk, b"before");
    assert_eq!(disk.writes, start);
}
