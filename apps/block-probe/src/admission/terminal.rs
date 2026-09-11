// SPDX-License-Identifier: Apache-2.0
use super::{content, control, open, request};
use crate::volume_disk::Disk;
use rustic_file_service::Server;
use rustic_fs::{AdmissionState as State, Error, PublicationPhase as Phase, Volume};

#[inline(never)]
pub(super) fn verify(disk: &mut Disk<'_>, phase: u64) {
    let mut s = Server::new(open(disk, phase));
    if phase == 1 {
        control::seed(&mut s, disk);
    }
    let caller = control::grant(&mut s, false);
    for key in [51, 52] {
        let old = s
            .volume
            .admission_by_retry(9, 4, request(&s.volume, key).retry)
            .unwrap()
            .status;
        assert_eq!(s.admission_status(caller, old.id, 1).unwrap(), old);
    }
    check(&mut s.volume, disk);
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
