// SPDX-License-Identifier: Apache-2.0
//! Role 14: mount the v6 workspace volume and read the artifact stored there
//! with a bounded buffer, then leave the digest it computed on the disk for the
//! host's independent reader (#51).
use crate::persistence::finish;
use crate::volume_disk::Disk;
use core::mem::MaybeUninit;
use rustic_fs::{Kind, VOLUME_SECTORS, Volume6};
use rustic_sdk::block::{Device, Operation};

/// The node table and the map are larger than this process's 64 KiB stack, so
/// the volume lives in the process's private static storage and is mounted in
/// place.
static mut VOLUME: MaybeUninit<Volume6> = MaybeUninit::uninit();

/// Where the host places the v6 volume, outside both v5 volumes.
const BASE: u64 = 1024;
/// The fixture artifact: far beyond a v5 file and spread over several runs.
const LENGTH: u32 = 16_384;
/// The guest's read buffer, so an artifact larger than its memory still reads.
const CHUNK: usize = 4096;
/// One sector before the volume (and inside the granted window), where the guest
/// leaves what it observed.
const EVIDENCE: u64 = BASE - 1;
const MAGIC: [u8; 8] = *b"RUSTICW1";

pub(super) fn run(device: &Device) -> u64 {
    let mut disk = Disk::at(device, BASE, VOLUME_SECTORS);
    // SAFETY: one thread per process, and the slot is written by this single
    // mount before any read and never aliased elsewhere.
    let volume = unsafe { &mut *core::ptr::addr_of_mut!(VOLUME) };
    let volume = volume.write(Volume6::EMPTY);
    if volume.mount_into(&mut disk).is_err() {
        return 0;
    }
    let Some(node) = volume.node(5) else {
        return 0;
    };
    if node.kind != Kind::File || node.name() != b"artifact" || node.length != LENGTH {
        return 0;
    }
    let mut digest = 0xcbf2_9ce4_8422_2325u64;
    let mut buffer = [0u8; CHUNK];
    let mut offset = 0u64;
    let mut ranges = 0u32;
    while offset < u64::from(LENGTH) {
        let want = (u64::from(LENGTH) - offset).min(CHUNK as u64) as usize;
        let Ok(taken) = volume.read_range(&mut disk, node, offset, &mut buffer[..want]) else {
            return 0;
        };
        if taken != want {
            return 0;
        }
        for byte in &buffer[..taken] {
            digest ^= u64::from(*byte);
            digest = digest.wrapping_mul(0x100_0000_01b3);
        }
        offset += taken as u64;
        ranges += 1;
    }
    let mut record = [0u8; 512];
    record[..8].copy_from_slice(&MAGIC);
    record[8..16].copy_from_slice(&u64::from(LENGTH).to_le_bytes());
    record[16..24].copy_from_slice(&digest.to_le_bytes());
    record[24..28].copy_from_slice(&ranges.to_le_bytes());
    let written = finish(device, device.write(EVIDENCE, &record).unwrap());
    if written.operation != Operation::Write {
        return 0;
    }
    let flushed = finish(device, device.flush().unwrap());
    if flushed.operation != Operation::Flush {
        return 0;
    }
    1
}
