// SPDX-License-Identifier: Apache-2.0
//! Initialization scratch must leave the stack before the long-lived dispatch loop.
#[inline(never)]
pub fn load(
    disk: &mut super::disk::Disk,
    initialize: bool,
) -> Result<rustic_fs::Volume, rustic_fs::Error> {
    if initialize {
        rustic_fs::Volume::initialize(disk)
    } else {
        rustic_fs::Volume::mount(disk)
    }
}
