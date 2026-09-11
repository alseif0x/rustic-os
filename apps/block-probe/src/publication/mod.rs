// SPDX-License-Identifier: Apache-2.0
//! Native publication mechanics; not a client/server cancellation endpoint.
mod verify;
use crate::volume_disk::Disk;
use rustic_fs::{Kind, Replacement, Retry, Volume};
use rustic_sdk::block::Device;

// Keep this fixture's volume buffers out of unrelated role dispatch frames.
#[inline(never)]
pub(super) fn run(device: &Device, expected: u64) -> u64 {
    let mut disk = Disk {
        device,
        writes: 0,
        base: 256,
    };
    let mut volume = if expected == 1 {
        let mut volume = Volume::initialize(&mut disk).unwrap();
        volume.enable_operations(&mut disk).unwrap();
        let file = volume
            .create(&mut disk, 4, b"publication", Kind::File)
            .unwrap();
        volume
            .replace(&mut disk, file.id, file.version, b"before")
            .unwrap();
        volume
    } else {
        assert_eq!(expected, 2);
        Volume::mount(&mut disk).unwrap()
    };
    let file = volume.lookup(4, b"publication").unwrap();
    let (lineage, epoch) = volume.recovery_info().unwrap();
    let retry = Retry {
        lineage,
        epoch,
        key: 42,
    };
    let previous = if expected == 1 {
        file.version
    } else {
        volume
            .operation_by_retry(9, 4, retry)
            .unwrap()
            .receipt
            .previous
    };
    let request = Replacement {
        workspace: 4,
        retry,
        id: file.id,
        version: previous,
    };
    if expected == 1 {
        verify::cancel_boundaries(&mut volume, &mut disk, request);
        verify::settle(&mut volume, &mut disk, request);
        16
    } else {
        verify::replay(&mut volume, &mut disk, request);
        0
    }
}
