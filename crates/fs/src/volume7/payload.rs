// SPDX-License-Identifier: Apache-2.0
//! Bounded streaming verification of v7 live and retained payload bytes.

use crate::checksum::crc_update;
use crate::format7::{NODES, Node7, RETAINED, Record7};
use crate::{Disk, Error, Kind};

use super::super::format7::PAYLOAD_SECTOR;

pub(super) fn verify_payloads(
    disk: &mut impl Disk,
    nodes: &[Node7; NODES],
    records: &[Option<Record7>; RETAINED],
) -> Result<(), Error> {
    for node in nodes {
        if node.kind == Kind::File {
            verify_payload(disk, node.runs(), node.length, node.payload_crc32)?;
        }
    }
    for record in records.iter().flatten() {
        verify_payload(disk, record.runs(), record.length, record.payload_crc32)?;
    }
    Ok(())
}

fn verify_payload(
    disk: &mut impl Disk,
    runs: &[crate::Extent],
    length: u32,
    expected: u32,
) -> Result<(), Error> {
    let mut remaining = length as usize;
    let mut checksum = !0u32;
    let mut block = [0u8; 512];
    for run in runs {
        for offset in 0..run.sectors {
            if remaining == 0 {
                break;
            }
            disk.read(PAYLOAD_SECTOR + run.start + offset, &mut block)?;
            let count = remaining.min(block.len());
            crc_update(&mut checksum, &block[..count]);
            remaining -= count;
        }
    }
    if remaining != 0 || !checksum != expected {
        return Err(Error::Corrupt);
    }
    Ok(())
}
