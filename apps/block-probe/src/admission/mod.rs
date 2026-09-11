// SPDX-License-Identifier: Apache-2.0
//! Native durable storage acceptance, without exposing a service endpoint.
mod pending;
mod terminal;
use crate::volume_disk::Disk;
use rustic_fs::{Kind, Replacement, Retry, Volume};
use rustic_sdk::block::Device;

pub(super) fn run(device: &Device, phase: u64, terminal: bool) {
    if terminal {
        terminal::verify(
            &mut Disk {
                device,
                writes: 0,
                base: 512,
            },
            phase,
        );
    } else {
        pending::verify(
            &mut Disk {
                device,
                writes: 0,
                base: 768,
            },
            phase,
        );
    }
}

fn open(disk: &mut Disk<'_>, phase: u64) -> Volume {
    if phase == 1 {
        let mut v = Volume::initialize(disk).unwrap();
        v.enable_operations(disk).unwrap();
        let file = v.create(disk, 4, b"admission", Kind::File).unwrap();
        v.replace(disk, file.id, file.version, b"before").unwrap();
        v.enable_admissions(disk).unwrap();
        v
    } else {
        assert_eq!(phase, 2);
        Volume::mount(disk).unwrap()
    }
}

fn request(v: &Volume, key: u64) -> Replacement {
    let file = v.lookup(4, b"admission").unwrap();
    let (lineage, epoch) = v.recovery_info().unwrap();
    Replacement {
        workspace: 4,
        retry: Retry {
            lineage,
            epoch,
            key,
        },
        id: file.id,
        version: file.version,
    }
}

fn content(v: &Volume, disk: &mut Disk<'_>, expected: &[u8]) {
    let mut bytes = [0; 1024];
    let file = v.lookup(4, b"admission").unwrap();
    let count = v.read(disk, file.id, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..count], expected);
}
