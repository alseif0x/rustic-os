// SPDX-License-Identifier: Apache-2.0
//! Native V7 durable storage acceptance through the existing file-service packets.
mod control;
mod disk;
mod pending;
mod setup;
mod terminal;
use crate::volume_disk::Disk;
use rustic_fs::{Volume7, WriteIdentity7, format7::Record7};
use rustic_sdk::abi::files::{
    operation::{Key, Replacement, Retry},
    reference::{Epoch, References, Version},
};
use rustic_sdk::block::Device;
use setup::open;

pub(super) fn run(device: &Device, phase: u64, terminal: bool) {
    if terminal {
        terminal::verify(
            &mut Disk::at(device, 131_539, rustic_fs::format7::VOLUME_SECTORS),
            phase,
        );
    } else {
        pending::verify(
            &mut Disk::at(device, 262_822, rustic_fs::format7::VOLUME_SECTORS),
            phase,
        );
    }
}

fn request(v: &Volume7, key: u64) -> Replacement {
    let file = v.lookup(4, b"admission").unwrap();
    let references = References::new(v.header().unwrap().lineage, 4, file.id).unwrap();
    Replacement {
        workspace: references.workspace,
        resource: references.resource,
        expected_version: Version::new(file.version).unwrap(),
        retry: Retry {
            epoch: Epoch::new(v.header().unwrap().epoch).unwrap(),
            key: Key::new(key).unwrap(),
        },
    }
}

fn identity(request: Replacement) -> WriteIdentity7 {
    WriteIdentity7 {
        subject: 9,
        workspace: request.workspace.root(),
        object: request.resource.object(),
        instance: 1,
        retry_epoch: request.retry.epoch.value(),
        retry_key: request.retry.key.value(),
    }
}

fn record(v: &Volume7, request: Replacement) -> Record7 {
    let identity = identity(request);
    *v.retained_records()
        .unwrap()
        .iter()
        .flatten()
        .find(|record| {
            record.subject == identity.subject
                && record.workspace == identity.workspace
                && record.retry_epoch == identity.retry_epoch
                && record.retry_key == identity.retry_key
        })
        .unwrap()
}

fn content(v: &Volume7, disk: &mut Disk<'_>, expected: &[u8]) {
    let mut bytes = [0; 1024];
    let file = v.lookup(4, b"admission").unwrap();
    let count = v.read_range(disk, file.id, None, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..count], expected);
}
