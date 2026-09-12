// SPDX-License-Identifier: Apache-2.0
//! Native durable storage acceptance, without exposing a service endpoint.
mod control;
mod disk;
mod pending;
mod setup;
mod terminal;
use crate::volume_disk::Disk;
use rustic_fs::{Replacement, Retry, Volume};
use rustic_sdk::block::Device;
use setup::open;

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
