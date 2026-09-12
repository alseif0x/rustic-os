// SPDX-License-Identifier: Apache-2.0
//! Valid-checksum corruptions must fail semantic validation, not just CRC checks.
use crate::{MemoryDisk, base};
use rustic_fs::{Error, Volume};

fn crc(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for b in bytes {
        c ^= u32::from(*b);
        for _ in 0..8 {
            c = (c >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(c & 1));
        }
    }
    !c
}

pub(super) fn rewrite(base: &MemoryDisk, change: impl Fn(&mut [u8])) -> MemoryDisk {
    let mut disk = base.recover(true);
    let bank = usize::from(
        u64::from_le_bytes(disk.live[13][12..20].try_into().unwrap())
            > u64::from_le_bytes(disk.live[8][12..20].try_into().unwrap()),
    );
    let mut header = disk.live[8 + bank * 5];
    let nodes = disk.live[9 + bank * 5..13 + bank * 5].to_vec();
    let mut records = disk.live[160 + bank * 7..167 + bank * 7].concat();
    change(&mut records);
    header[32..36].copy_from_slice(&crc(&records).to_le_bytes());
    header[28..32].fill(0);
    let checksum = crc(&header);
    header[28..32].copy_from_slice(&checksum.to_le_bytes());
    for bank in 0..2 {
        disk.live[8 + bank * 5] = header;
        disk.live[9 + bank * 5..13 + bank * 5].copy_from_slice(&nodes);
        for (sector, bytes) in records.as_chunks::<512>().0.iter().enumerate() {
            disk.live[160 + bank * 7 + sector] = *bytes;
        }
    }
    disk
}

#[test]
fn valid_checksums_cannot_hide_forged_admission_states_or_colliding_ids() {
    let (mut disk, request) = base();
    let mut v = Volume::mount(&mut disk).unwrap();
    v.enable_admissions(&mut disk).unwrap();
    let first = v.admit_replace(&mut disk, 9, 0, request, b"first").unwrap();
    let second = v
        .admit_replace(&mut disk, 10, 0, request, b"second")
        .unwrap();
    v.cancel_admission(&mut disk, 9, first.id).unwrap();
    Volume::mount(&mut rewrite(&disk, |_| ())).unwrap();
    // Corrupt the already cancelled record while recomputing all checksums.
    for (offset, value) in [
        (64, 0),
        (72, 0),
        (80, 0),
        (80, 1),
        (80, 3),
        (80, 4),
        (81, 1),
        (511, 1),
        (48, 0),
    ] {
        let mut bad = rewrite(&disk, |r| r[512 + offset] = value);
        assert!(
            matches!(Volume::mount(&mut bad), Err(Error::Corrupt)),
            "offset={offset} value={value}"
        );
    }
    for number in [
        first.id.number,
        second.id.number,
        v.sequence(),
        v.sequence() + 1,
    ] {
        let mut bad = rewrite(&disk, |r| {
            r[512 + 1536 + 64..512 + 1536 + 72].copy_from_slice(&number.to_le_bytes())
        });
        if number == second.id.number {
            Volume::mount(&mut bad).unwrap();
        } else {
            assert!(matches!(Volume::mount(&mut bad), Err(Error::Corrupt)));
        }
    }
    // Old decoders' reserved bytes are not silently reused under an old magic.
    let mut bad = rewrite(&disk, |r| r[..8].copy_from_slice(b"RUSTREC2"));
    assert!(matches!(Volume::mount(&mut bad), Err(Error::Corrupt)));
}
