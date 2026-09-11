// SPDX-License-Identifier: Apache-2.0
use crate::volume_disk::Disk;
use rustic_fs::{
    Error, PublicationCancel as Cancel, PublicationPhase as Phase, Replacement, Volume,
};

fn content(volume: &Volume, disk: &mut Disk<'_>, id: u32, expected: &[u8]) {
    let mut bytes = [0; 1024];
    let count = volume.read(disk, id, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..count], expected);
}

pub(super) fn cancel_boundaries(volume: &mut Volume, disk: &mut Disk<'_>, request: Replacement) {
    let sequence = volume.sequence();
    for boundary in 0..=15 {
        let start = disk.writes;
        {
            let mut write = volume
                .prepare_scoped(disk, 9, 0, request, b"after")
                .unwrap();
            for _ in 0..boundary {
                write.advance().unwrap();
            }
            assert_eq!(write.result(), None);
            assert_eq!(write.cancel(), Ok(Cancel::Cancelled));
            assert_eq!(write.advance(), Ok(Phase::Cancelled));
            assert_eq!(write.cancel(), Ok(Cancel::Cancelled));
        }
        assert_eq!(disk.writes - start, boundary);
        assert_eq!(volume.sequence(), sequence);
        assert_eq!(volume.stat(request.id).unwrap().version, request.version);
        content(volume, disk, request.id, b"before");
        assert!(matches!(
            volume.operation_by_retry(9, 4, request.retry),
            Err(Error::OutcomeUnknown)
        ));
    }
}

pub(super) fn settle(volume: &mut Volume, disk: &mut Disk<'_>, request: Replacement) {
    let start = disk.writes;
    let receipt;
    {
        let mut write = volume
            .prepare_scoped(disk, 9, 0, request, b"after")
            .unwrap();
        for _ in 0..15 {
            write.advance().unwrap();
        }
        assert_eq!(write.phase(), Phase::ReadyToPublish);
        assert_eq!(write.advance(), Ok(Phase::Settling));
        assert_eq!(write.result(), None);
        assert_eq!(write.cancel(), Ok(Cancel::TooLate));
        assert_eq!(write.advance(), Ok(Phase::Committed));
        receipt = write.result().unwrap();
        assert_eq!(write.cancel(), Ok(Cancel::TooLate));
        assert_eq!(write.advance(), Ok(Phase::Committed));
    }
    assert_eq!(disk.writes - start, 17);
    content(volume, disk, request.id, b"after");
    assert_eq!(
        volume
            .operation_by_retry(9, 4, request.retry)
            .unwrap()
            .receipt,
        receipt
    );
}

pub(super) fn replay(volume: &mut Volume, disk: &mut Disk<'_>, request: Replacement) {
    let receipt = volume
        .operation_by_retry(9, 4, request.retry)
        .unwrap()
        .receipt;
    content(volume, disk, request.id, b"after");
    let start = disk.writes;
    {
        let mut write = volume
            .prepare_scoped(disk, 9, 0, request, b"after")
            .unwrap();
        assert_eq!(write.phase(), Phase::Committed);
        assert_eq!(write.result(), Some(receipt));
        assert_eq!(write.cancel(), Ok(Cancel::TooLate));
        assert_eq!(write.advance(), Ok(Phase::Committed));
    }
    assert_eq!(disk.writes, start);
}
