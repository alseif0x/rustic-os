// SPDX-License-Identifier: Apache-2.0
use super::{content, open, request};
use crate::volume_disk::Disk;
use rustic_fs::{
    AdmissionState as State, Error, PublicationCancel as Cancel, PublicationPhase as Phase, Volume,
};

#[inline(never)]
pub(super) fn verify(disk: &mut Disk<'_>, phase: u64) {
    let mut v = open(disk, phase);
    if phase == 1 {
        seed(&mut v, disk);
    }
    check(&mut v, disk);
}

// Separate setup and replay buffers on the native 64 KiB stack. The second VM
// performs the actual remount; replay must not retain a second volume here.
#[inline(never)]
fn seed(v: &mut Volume, disk: &mut Disk<'_>) {
    let a = v
        .admit_replace(disk, 9, 0, request(v, 51), b"cancelled bytes")
        .unwrap();
    {
        let mut write = v.prepare_admitted(disk, 9, a.id).unwrap();
        for _ in 0..15 {
            write.advance().unwrap();
        }
        assert_eq!(write.cancel(), Ok(Cancel::Cancelled));
    }
    assert_eq!(
        v.admission_by_id(9, a.id).unwrap().status.state,
        State::Admitted
    );
    assert_eq!(
        v.cancel_admission(disk, 9, a.id).unwrap().state,
        State::Cancelled
    );
    content(v, disk, b"before");
    let b = v
        .admit_replace(disk, 9, a.id.number, request(v, 52), b"after")
        .unwrap();
    let mut write = v.prepare_admitted(disk, 9, b.id).unwrap();
    for _ in 0..16 {
        write.advance().unwrap();
    }
    assert_eq!(write.cancel(), Ok(Cancel::TooLate));
    assert_eq!(write.advance(), Ok(Phase::Committed));
    assert!(write.result().unwrap().committed > b.id.number);
}

#[inline(never)]
fn check(v: &mut Volume, disk: &mut Disk<'_>) {
    content(v, disk, b"after");
    let start = disk.writes;
    for (key, state) in [(51, State::Cancelled), (52, State::Committed)] {
        let a = v.admission_by_retry(9, 4, request(v, key).retry).unwrap();
        assert_eq!(a.status.state, state);
        let (status, request, receipt) = (a.status, a.request, a.receipt);
        assert!(matches!(
            v.admission_by_id(8, status.id),
            Err(Error::OutcomeUnknown)
        ));
        assert_eq!(v.cancel_admission(disk, 9, status.id).unwrap(), status);
        let bytes = if key == 51 {
            b"cancelled bytes".as_slice()
        } else {
            b"after"
        };
        assert_eq!(v.admit_replace(disk, 9, 0, request, bytes).unwrap(), status);
        assert_eq!(
            v.admit_replace(disk, 9, 0, request, b"conflict"),
            Err(Error::IdempotencyConflict)
        );
        if key == 51 {
            assert!(matches!(
                v.prepare_admitted(disk, 9, status.id),
                Err(Error::Cancelled)
            ));
        } else {
            let mut replay = v.prepare_admitted(disk, 9, status.id).unwrap();
            assert_eq!(replay.advance(), Ok(Phase::Committed));
            assert_eq!(replay.result(), receipt);
        }
    }
    assert_eq!(disk.writes, start);
}
