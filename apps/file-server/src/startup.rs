// SPDX-License-Identifier: Apache-2.0
//! Initialization scratch must leave the stack before the long-lived dispatch loop.
static mut VOLUME7: rustic_fs::Volume7 = rustic_fs::Volume7::EMPTY;

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

/// Mount V7 into process-owned static storage; its decoded tables exceed the
/// native application's bounded stack. The exclusive borrow is handed to the
/// V7 service, which owns every read and mutation for the process lifetime.
#[inline(never)]
pub fn mount_v7(
    disk: &mut super::disk::Disk,
) -> Result<&'static mut rustic_fs::Volume7, rustic_fs::Error> {
    // SAFETY: the file-server entry point calls this at most once in its
    // single-threaded process, and nothing else names `VOLUME7`, so the
    // returned exclusive borrow is the only reference for the process lifetime.
    let volume = unsafe { &mut *core::ptr::addr_of_mut!(VOLUME7) };
    volume.mount_into(disk)?;
    Ok(volume)
}
