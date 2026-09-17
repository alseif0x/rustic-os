// SPDX-License-Identifier: Apache-2.0
//! Role 14: mount the v6 workspace volume, read the artifact stored there with a
//! bounded buffer, and on the first boot publish a tracked write whose receipt
//! the second boot must find, verify and replay without writing (#51).
use crate::persistence::finish;
use crate::volume_disk::Disk;
use core::mem::MaybeUninit;
use rustic_fs::{Kind, Node6, Retry, VOLUME_SECTORS, Volume6};
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
const GUEST_NAME: &[u8] = b"guest";
const RETRY_KEY: u64 = 21;

pub(super) fn run(device: &Device, phase: u64) -> u64 {
    let mut disk = Disk::at(device, BASE, VOLUME_SECTORS);
    // SAFETY: one thread per process, and the slot is written by this single
    // mount before any read and never aliased elsewhere.
    let volume = unsafe { &mut *core::ptr::addr_of_mut!(VOLUME) };
    let volume = volume.write(Volume6::EMPTY);
    if volume.mount_into(&mut disk).is_err() {
        return 0;
    }
    let Some(artifact) = volume.node(5) else {
        return 0;
    };
    if artifact.kind != Kind::File || artifact.name() != b"artifact" || artifact.length != LENGTH {
        return 0;
    }
    let Some((digest, ranges)) = read_artifact(volume, &mut disk, artifact) else {
        return 0;
    };
    let payload = digest.to_le_bytes();
    let retry = Retry {
        lineage: volume.receipts.lineage(),
        epoch: volume.receipts.epoch(),
        key: RETRY_KEY,
    };
    let mut record = [0u8; 512];
    record[..8].copy_from_slice(&MAGIC);
    record[8..16].copy_from_slice(&u64::from(LENGTH).to_le_bytes());
    record[16..24].copy_from_slice(&digest.to_le_bytes());
    record[24..28].copy_from_slice(&ranges.to_le_bytes());
    record[28] = phase as u8;
    let receipt = if phase == 1 {
        let Some(slot) = volume
            .nodes
            .iter()
            .position(|node| node.kind == Kind::Empty)
        else {
            return 0;
        };
        let mut node = Node6::EMPTY;
        node.id = volume.nodes.iter().map(|node| node.id).max().unwrap_or(0) + 1;
        node.parent = 4;
        node.kind = Kind::File;
        node.version = 1;
        node.name[..GUEST_NAME.len()].copy_from_slice(GUEST_NAME);
        node.name_length = GUEST_NAME.len() as u8;
        volume.nodes[slot] = node;
        let Ok(receipt) = volume.write_tracked(&mut disk, slot, 1, retry, &payload) else {
            return 0;
        };
        receipt
    } else {
        // After the reboot the receipt and the bytes it names must still be
        // there, and replaying the same retry must return the retained outcome
        // without writing anything.
        let Some(node) = volume
            .nodes
            .iter()
            .find(|node| node.kind == Kind::File && node.name() == GUEST_NAME)
        else {
            return 0;
        };
        let Ok(Some(retained)) = volume.find_receipt(retry) else {
            return 0;
        };
        let retained = *retained;
        if node.id != retained.id || node.version != retained.committed {
            return 0;
        }
        let mut observed = [0u8; 8];
        if volume.read_range(&mut disk, node, 0, &mut observed) != Ok(8) {
            return 0;
        }
        if u64::from_le_bytes(observed) != digest {
            return 0;
        }
        let Ok(replayed) = volume.write_tracked(&mut disk, 5, retained.previous, retry, &payload)
        else {
            return 0;
        };
        if replayed != retained {
            return 0;
        }
        retained
    };
    record[32..36].copy_from_slice(&receipt.id.to_le_bytes());
    record[36..44].copy_from_slice(&receipt.committed.to_le_bytes());
    record[44..48].copy_from_slice(&receipt.length.to_le_bytes());
    record[48] = phase as u8;
    write_evidence(device, record)
}

/// Fold the artifact into one digest through 4 KiB ranges, so the whole file is
/// never held at once.
fn read_artifact(volume: &Volume6, disk: &mut Disk<'_>, node: &Node6) -> Option<(u64, u32)> {
    let mut digest = 0xcbf2_9ce4_8422_2325u64;
    let mut buffer = [0u8; CHUNK];
    let mut offset = 0u64;
    let mut ranges = 0u32;
    while offset < u64::from(LENGTH) {
        let want = (u64::from(LENGTH) - offset).min(CHUNK as u64) as usize;
        let taken = volume
            .read_range(disk, node, offset, &mut buffer[..want])
            .ok()?;
        if taken != want {
            return None;
        }
        for byte in &buffer[..taken] {
            digest ^= u64::from(*byte);
            digest = digest.wrapping_mul(0x100_0000_01b3);
        }
        offset += taken as u64;
        ranges += 1;
    }
    Some((digest, ranges))
}

fn write_evidence(device: &Device, record: [u8; 512]) -> u64 {
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
