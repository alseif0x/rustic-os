// SPDX-License-Identifier: Apache-2.0
mod support;
use rustic_fs::{Error, Kind, Volume};
use support::MemoryDisk;

#[test]
fn workspace_and_resource_ids_survive_remount_but_never_recreation() {
    let mut disk = MemoryDisk::new();
    let mut volume = Volume::initialize(&mut disk).unwrap();
    volume.enable_recovery(&mut disk, [7; 16]).unwrap();
    let workspace = volume
        .create(&mut disk, 4, b"project", Kind::Directory)
        .unwrap();
    let file = volume
        .create(&mut disk, workspace.id, b"file", Kind::File)
        .unwrap();
    let file = volume
        .replace(&mut disk, file.id, file.version, b"before")
        .unwrap();
    disk = disk.recover(true);
    let mut volume = Volume::mount(&mut disk).unwrap();
    assert_eq!(volume.recovery_info().unwrap().0, [7; 16]);
    let same = volume.resolve(workspace.id, file.id).unwrap();
    assert_eq!((same.id, same.version), (file.id, file.version));

    volume.remove(&mut disk, file.id).unwrap();
    let replacement = volume
        .create(&mut disk, workspace.id, b"file", Kind::File)
        .unwrap();
    assert!(replacement.id > file.id);
    assert!(matches!(
        volume.resolve(workspace.id, file.id),
        Err(Error::NotFound)
    ));
    volume.remove(&mut disk, replacement.id).unwrap();
    volume.remove(&mut disk, workspace.id).unwrap();
    let fresh_workspace = volume
        .create(&mut disk, 4, b"project", Kind::Directory)
        .unwrap();
    let fresh_file = volume
        .create(&mut disk, fresh_workspace.id, b"file", Kind::File)
        .unwrap();
    assert!(fresh_workspace.id > workspace.id && fresh_file.id > replacement.id);
    let volume = Volume::mount(&mut disk).unwrap();
    assert!(volume.resolve(fresh_workspace.id, fresh_file.id).is_ok());
    assert!(matches!(
        volume.resolve(workspace.id, fresh_file.id),
        Err(Error::NotFound)
    ));
    assert!(matches!(volume.stat(file.id), Err(Error::NotFound)));
}

#[test]
fn resolution_requires_a_live_directory_and_actual_ancestry() {
    let mut disk = MemoryDisk::new();
    let mut volume = Volume::initialize(&mut disk).unwrap();
    let a = volume.create(&mut disk, 4, b"a", Kind::Directory).unwrap();
    let b = volume.create(&mut disk, 4, b"b", Kind::Directory).unwrap();
    let sub = volume
        .create(&mut disk, a.id, b"nested", Kind::Directory)
        .unwrap();
    let file = volume
        .create(&mut disk, sub.id, b"file", Kind::File)
        .unwrap();
    assert!(volume.resolve(a.id, file.id).is_ok());
    assert!(volume.resolve(sub.id, file.id).is_ok());
    assert!(volume.resolve(4, file.id).is_ok());
    assert!(matches!(
        volume.resolve(b.id, file.id),
        Err(Error::NotFound)
    ));
    assert!(matches!(volume.resolve(2, file.id), Err(Error::NotFound)));
    assert!(matches!(
        volume.resolve(file.id, file.id),
        Err(Error::NotDirectory)
    ));
    assert!(matches!(volume.resolve(0, file.id), Err(Error::NotFound)));
    assert!(matches!(
        volume.resolve(a.id, u32::MAX),
        Err(Error::NotFound)
    ));
}

#[test]
fn versioned_ranges_publish_matching_metadata_and_leave_errors_untouched() {
    let mut disk = MemoryDisk::new();
    let mut volume = Volume::initialize(&mut disk).unwrap();
    let file = volume.create(&mut disk, 2, b"binary", Kind::File).unwrap();
    let mut bytes = [0xaa; 4];
    let (empty, count) = volume
        .read_versioned(&mut disk, file.id, None, 0, &mut bytes)
        .unwrap();
    assert_eq!((empty.version, count), (file.version, 0));
    assert_eq!(bytes, [0xaa; 4]);
    let changed = volume
        .replace(&mut disk, file.id, file.version, &[0, 0xff, 0x80, 3, 4])
        .unwrap();
    assert!(matches!(
        volume.read_versioned(&mut disk, file.id, Some(file.version), 0, &mut bytes),
        Err(Error::Version)
    ));
    assert_eq!(bytes, [0xaa; 4]);
    let (observed, count) = volume
        .read_versioned(&mut disk, file.id, Some(changed.version), 2, &mut bytes)
        .unwrap();
    assert_eq!(
        (observed.version, observed.length, count),
        (changed.version, 5, 3)
    );
    assert_eq!(bytes, [0x80, 3, 4, 0xaa]);
    let before = bytes;
    assert!(matches!(
        volume.read_versioned(&mut disk, file.id, None, 6, &mut bytes),
        Err(Error::Size)
    ));
    assert_eq!(bytes, before);
    assert_eq!(
        volume
            .read_versioned(&mut disk, file.id, None, 5, &mut bytes)
            .unwrap()
            .1,
        0
    );
    assert_eq!(bytes, before);
}
