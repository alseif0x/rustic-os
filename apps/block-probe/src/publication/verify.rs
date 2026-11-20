// SPDX-License-Identifier: Apache-2.0
use crate::volume_disk::Disk;
use core::task::Poll;
use rustic_fs::{
    Publication7Cancel as Cancel, Publication7Phase as Phase, Stage7, Stage7Kind, Volume7,
    WriteIdentity7, format7::Record7,
};

const BOUNDARIES: [usize; 16] = [0, 1, 2, 3, 7, 15, 31, 63, 64, 65, 79, 95, 96, 99, 100, 101];
const PRE_HEADER_COMMANDS: usize = 101;

pub(super) fn record(volume: &Volume7, identity: WriteIdentity7) -> Option<Record7> {
    volume
        .retained_records()
        .unwrap()
        .iter()
        .flatten()
        .find(|record| {
            record.subject == identity.subject
                && record.workspace == identity.workspace
                && record.retry_epoch == identity.retry_epoch
                && record.retry_key == identity.retry_key
        })
        .copied()
}

fn content(volume: &Volume7, disk: &mut Disk<'_>, id: u32, expected: &[u8]) {
    let mut bytes = [0; 1024];
    let count = volume.read_range(disk, id, None, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..count], expected);
}

fn stage(
    volume: &mut Volume7,
    disk: &mut Disk<'_>,
    identity: WriteIdentity7,
    previous: u64,
) -> Stage7 {
    let mut stage = volume
        .open_stage(identity, previous, 5, Stage7Kind::Tracked)
        .unwrap();
    assert_eq!(volume.stage_write(disk, &mut stage, b"after"), Ok(0));
    stage
}

pub(super) fn cancel_boundaries(
    volume: &mut Volume7,
    disk: &mut Disk<'_>,
    identity: WriteIdentity7,
    previous: u64,
) {
    let sequence = volume.header().unwrap().sequence;
    for boundary in BOUNDARIES {
        let mut stage = stage(volume, disk, identity, previous);
        let start = disk.events;
        {
            let mut write = volume.finish_tracked_poll(disk, &mut stage).unwrap();
            for _ in 0..boundary {
                assert!(matches!(write.poll_advance(), Poll::Ready(Ok(_))));
            }
            if boundary == 101 {
                assert_eq!(write.phase(), Phase::ReadyToPublish);
            }
            assert_eq!(write.result(), None);
            assert_eq!(write.abort_before_header(), Ok(Cancel::Cancelled));
            assert_eq!(write.poll_advance(), Poll::Ready(Ok(Phase::Cancelled)));
            assert_eq!(write.abort_before_header(), Ok(Cancel::Cancelled));
            drop(write);
        }
        assert_eq!(disk.events - start, boundary);
        assert_eq!(volume.header().unwrap().sequence, sequence);
        assert_eq!(volume.stat(identity.object).unwrap().version, previous);
        content(volume, disk, identity.object, b"before");
        assert_eq!(record(volume, identity), None);
    }
}

pub(super) fn settle(
    volume: &mut Volume7,
    disk: &mut Disk<'_>,
    identity: WriteIdentity7,
    previous: u64,
) {
    let mut stage = stage(volume, disk, identity, previous);
    let start = disk.events;
    let receipt;
    {
        let mut write = volume.finish_tracked_poll(disk, &mut stage).unwrap();
        let mut before_header = 0;
        while write.phase() != Phase::ReadyToPublish {
            assert!(matches!(write.poll_advance(), Poll::Ready(Ok(_))));
            before_header += 1;
        }
        assert_eq!(before_header, PRE_HEADER_COMMANDS);
        assert_eq!(write.phase(), Phase::ReadyToPublish);
        assert_eq!(write.poll_advance(), Poll::Ready(Ok(Phase::Settling)));
        assert_eq!(write.result(), None);
        assert_eq!(write.abort_before_header(), Ok(Cancel::TooLate));
        assert_eq!(write.poll_advance(), Poll::Ready(Ok(Phase::Committed)));
        receipt = write.result().unwrap();
        assert_eq!(write.abort_before_header(), Ok(Cancel::TooLate));
        assert_eq!(write.poll_advance(), Poll::Ready(Ok(Phase::Committed)));
        drop(write);
    }
    assert_eq!(disk.events - start, PRE_HEADER_COMMANDS + 2);
    content(volume, disk, identity.object, b"after");
    assert_eq!(record(volume, identity), Some(receipt));
}

pub(super) fn replay(
    volume: &mut Volume7,
    disk: &mut Disk<'_>,
    identity: WriteIdentity7,
    previous: u64,
) {
    let receipt = record(volume, identity).unwrap();
    content(volume, disk, identity.object, b"after");
    let start = disk.writes;
    let mut stage = stage(volume, disk, identity, previous);
    {
        let mut write = volume.finish_tracked_poll(disk, &mut stage).unwrap();
        assert_eq!(write.phase(), Phase::Committed);
        assert_eq!(write.result(), Some(receipt));
        assert_eq!(write.abort_before_header(), Ok(Cancel::TooLate));
        assert_eq!(write.poll_advance(), Poll::Ready(Ok(Phase::Committed)));
    }
    assert_eq!(disk.writes, start);
}
