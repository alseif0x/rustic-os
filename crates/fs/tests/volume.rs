// SPDX-License-Identifier: Apache-2.0
mod support;
use rustic_fs::{Error, Kind, MAX_FILE, OBJECTS, Volume};
use support::MemoryDisk;
#[test]
fn create_modify_reopen_and_namespace_separation() {
    let mut disk = MemoryDisk::new();
    let mut fs = Volume::initialize(&mut disk).unwrap();
    assert!(matches!(Volume::initialize(&mut disk), Err(Error::Corrupt)));
    let dir = fs
        .create(&mut disk, 4, b"project", Kind::Directory)
        .unwrap();
    let a = fs
        .create(&mut disk, dir.id, b"notes.txt", Kind::File)
        .unwrap();
    let data = fs.create(&mut disk, 2, b"notes.txt", Kind::File).unwrap();
    assert!(fs.within(a.id, dir.id));
    assert!(!fs.within(data.id, dir.id));
    assert!(!fs.within(999, 999));
    assert!(matches!(
        fs.create(&mut disk, 1, b"base", Kind::File),
        Err(Error::ReadOnly)
    ));
    let b = fs
        .replace(&mut disk, a.id, a.version, b"persistent content")
        .unwrap();
    assert!(matches!(
        fs.replace(&mut disk, b.id, a.version, b"stale"),
        Err(Error::Version)
    ));
    disk = disk.recover(true);
    let fs = Volume::mount(&mut disk).unwrap();
    let found = fs.lookup(dir.id, b"notes.txt").unwrap();
    assert_eq!(found.version, b.version);
    let mut bytes = [0; 64];
    let count = fs.read(&mut disk, found.id, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..count], b"persistent content");
    assert_eq!(fs.lookup(2, b"notes.txt").unwrap().length, 0);
    assert_eq!(disk.live[0], [0; 512]);
}
#[test]
fn names_capacity_and_identity_do_not_weaken_after_deletion() {
    let mut disk = MemoryDisk::new();
    let mut fs = Volume::initialize(&mut disk).unwrap();
    for name in [
        b"".as_slice(),
        b"..",
        b".",
        b"a/b",
        b"a\0x",
        b"abcdefghijklmnopqrstuvwxyz123456",
    ] {
        assert!(matches!(
            fs.create(&mut disk, 4, name, Kind::File),
            Err(Error::Invalid)
        ));
    }
    let mut first = 0;
    for i in 0..OBJECTS - 4 {
        let n = fs
            .create(&mut disk, 4, format!("file{i}").as_bytes(), Kind::File)
            .unwrap();
        if i == 0 {
            first = n.id;
        }
    }
    let before = disk.operations;
    assert!(matches!(
        fs.create(&mut disk, 4, b"full", Kind::File),
        Err(Error::Full)
    ));
    assert_eq!(disk.operations, before);
    fs.remove(&mut disk, first).unwrap();
    let node = fs.create(&mut disk, 4, b"replacement", Kind::File).unwrap();
    assert!(node.id > first);
    assert!(matches!(fs.stat(first), Err(Error::NotFound)));
    assert!(matches!(
        fs.replace(&mut disk, node.id, node.version, &[0; MAX_FILE + 1]),
        Err(Error::Size)
    ));
    assert_eq!(fs.remove(&mut disk, 4), Err(Error::ReadOnly));
}
#[test]
fn corruption_never_leaks_a_partial_read_or_formats_existing_data() {
    let mut disk = MemoryDisk::new();
    let mut fs = Volume::initialize(&mut disk).unwrap();
    let dir = fs.create(&mut disk, 4, b"dir", Kind::Directory).unwrap();
    let f = fs.create(&mut disk, dir.id, b"a", Kind::File).unwrap();
    assert_eq!(fs.remove(&mut disk, dir.id), Err(Error::NotEmpty));
    fs.replace(&mut disk, f.id, f.version, b"hello").unwrap();
    // File occupies metadata slot 5, inactive extent 1 at sector 54.
    disk.live[54][0] ^= 1;
    let mut output = [0xaa; 16];
    assert_eq!(
        fs.read(&mut disk, f.id, 0, &mut output),
        Err(Error::Corrupt)
    );
    assert_eq!(output, [0xaa; 16]);
    disk.live[8][0] = 0xff;
    disk.live[13][0] = 0xff;
    assert!(matches!(Volume::mount(&mut disk), Err(Error::Corrupt)));
    let before = disk.live.clone();
    assert!(Volume::initialize(&mut disk).is_err());
    assert_eq!(disk.live, before);
}
