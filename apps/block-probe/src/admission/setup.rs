// SPDX-License-Identifier: Apache-2.0
//! Mount the host-provisioned V7 fixture in process-owned long-lived storage.
use crate::volume_disk::{Disk, mount};
use rustic_fs::{Kind, Volume7};

pub(super) fn open(disk: &mut Disk<'_>, phase: u64) -> &'static mut Volume7 {
    let volume = mount(disk);
    if phase == 1 {
        initialize(disk, volume);
    } else {
        assert_eq!(phase, 2);
    }
    volume
}

#[inline(never)]
fn initialize(disk: &mut Disk<'_>, volume: &mut Volume7) {
    let file = volume.create(disk, 4, b"admission", Kind::File).unwrap();
    volume
        .replace(disk, file.id, file.version, b"before")
        .unwrap();
}
