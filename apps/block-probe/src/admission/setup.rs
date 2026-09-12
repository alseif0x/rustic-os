// SPDX-License-Identifier: Apache-2.0
//! Keep initial provisioning workspaces off the replay mount's bounded stack.
use crate::volume_disk::Disk;
use rustic_fs::{Kind, Volume};

pub(super) fn open(disk: &mut Disk<'_>, phase: u64) -> Volume {
    if phase == 1 {
        initialize(disk)
    } else {
        assert_eq!(phase, 2);
        Volume::mount(disk).unwrap()
    }
}

// Initial mutations and replay mount have separate reasons for change and large
// temporary values. Do not merge their stack frames through inlining.
#[inline(never)]
fn initialize(disk: &mut Disk<'_>) -> Volume {
    let mut v = Volume::initialize(disk).unwrap();
    v.enable_operations(disk).unwrap();
    let file = v.create(disk, 4, b"admission", Kind::File).unwrap();
    v.replace(disk, file.id, file.version, b"before").unwrap();
    v.enable_admissions(disk).unwrap();
    v
}
