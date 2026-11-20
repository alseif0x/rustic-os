// SPDX-License-Identifier: Apache-2.0
//! Native V7 staged publication mechanics; not a client/server endpoint.
mod verify;
use crate::volume_disk::{Disk, mount};
use rustic_fs::{Kind, WriteIdentity7};
use rustic_sdk::block::Device;

#[inline(never)]
pub(super) fn run(device: &Device, expected: u64) -> u64 {
    let mut disk = Disk::at(device, 256, rustic_fs::format7::VOLUME_SECTORS);
    let volume = mount(&mut disk);
    let file = if expected == 1 {
        let file = volume
            .create(&mut disk, 4, b"publication", Kind::File)
            .unwrap();
        volume
            .replace(&mut disk, file.id, file.version, b"before")
            .unwrap();
        file
    } else {
        assert_eq!(expected, 2);
        volume.lookup(4, b"publication").unwrap()
    };
    let identity = WriteIdentity7 {
        subject: 9,
        workspace: 4,
        object: file.id,
        instance: 1,
        retry_epoch: volume.header().unwrap().epoch,
        retry_key: 42,
    };
    let previous = if expected == 1 {
        volume.stat(file.id).unwrap().version
    } else {
        verify::record(volume, identity).unwrap().previous
    };
    if expected == 1 {
        verify::cancel_boundaries(volume, &mut disk, identity, previous);
        verify::settle(volume, &mut disk, identity, previous);
        16
    } else {
        verify::replay(volume, &mut disk, identity, previous);
        0
    }
}
