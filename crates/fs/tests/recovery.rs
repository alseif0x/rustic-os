// SPDX-License-Identifier: Apache-2.0
mod support;
use rustic_fs::{Error, Kind, Volume};
use support::MemoryDisk;
#[test]
fn every_write_flush_and_torn_sector_cut_recovers_old_or_new_data() {
    let mut base = MemoryDisk::new();
    let mut volume = Volume::initialize(&mut base).unwrap();
    let file = volume.create(&mut base, 4, b"atomic", Kind::File).unwrap();
    let old = [0x31; 1024];
    let new = [0x72; 1024];
    let file = volume
        .replace(&mut base, file.id, file.version, &old)
        .unwrap();
    for tear in [0, 1, 16, 32, 256, 511] {
        for cut in 0..=10 {
            let mut disk = base.recover(true);
            let mut volume = Volume::mount(&mut disk).unwrap();
            disk.operations = 0;
            disk.fail = Some(cut);
            disk.tear = tear;
            let result = volume.replace(&mut disk, file.id, file.version, &new);
            if cut < 10 {
                assert!(matches!(result, Err(Error::Uncertain)));
                assert!(matches!(volume.stat(file.id), Err(Error::Uncertain)));
            } else {
                assert!(result.is_ok());
            }
            for durable in [false, true] {
                let mut recovered = disk.recover(durable);
                let mounted = Volume::mount(&mut recovered).unwrap();
                let mut bytes = [0; 1024];
                assert_eq!(
                    mounted.read(&mut recovered, file.id, 0, &mut bytes),
                    Ok(1024)
                );
                assert!(
                    bytes == old || bytes == new,
                    "cut={cut} tear={tear} durable={durable}"
                );
                if result.is_ok() {
                    assert_eq!(bytes, new);
                }
            }
        }
    }
}
#[test]
fn malformed_media_is_bounded_and_does_not_panic() {
    for seed in 0..128u8 {
        let mut disk = MemoryDisk::new();
        for (i, b) in disk.live[8].iter_mut().enumerate() {
            *b = seed.wrapping_add(i as u8).wrapping_mul(31);
        }
        disk.live[13] = disk.live[8];
        assert!(Volume::mount(&mut disk).is_err());
    }
}
