// SPDX-License-Identifier: Apache-2.0
//! Bounded streaming verification of v7 live and retained payload bytes.

use crate::checksum::{crc, crc_update};
use crate::extent::{DATA_SECTORS, Extent};
use crate::format7::{MAX_EXTENTS, MAX_FILE_BYTES, NODES, Node7, RETAINED, Record7};
use crate::{Disk, Error, Kind};

use super::super::format7::PAYLOAD_SECTOR;

#[derive(Clone, Copy)]
pub(super) struct PayloadPlan {
    pub(super) runs: [Extent; MAX_EXTENTS],
    pub(super) used: usize,
}

impl PayloadPlan {
    pub(super) fn empty() -> Self {
        Self {
            runs: [Extent::new(0, 0); MAX_EXTENTS],
            used: 0,
        }
    }

    pub(super) fn runs(&self) -> &[Extent] {
        &self.runs[..self.used]
    }
}

/// Choose at most eight free runs without changing the mounted allocation map.
pub(super) fn plan_payload(map: &[u64], length: usize) -> Result<PayloadPlan, Error> {
    if length > MAX_FILE_BYTES as usize || map.len() != crate::extent::MAP_WORDS {
        return Err(Error::Size);
    }
    let mut remaining = (length as u64).div_ceil(512);
    let mut plan = PayloadPlan::empty();
    while remaining > 0 {
        if plan.used == MAX_EXTENTS {
            return Err(Error::Full);
        }

        let mut best = None;
        let mut cursor = 0;
        while cursor < DATA_SECTORS {
            while cursor < DATA_SECTORS && allocated(map, cursor) {
                cursor += 1;
            }
            let start = cursor;
            while cursor < DATA_SECTORS && !allocated(map, cursor) {
                cursor += 1;
            }
            let sectors = cursor - start;
            if sectors > 0
                && !plan
                    .runs()
                    .iter()
                    .any(|selected| start < selected.end() && selected.start < cursor)
                && best.is_none_or(|current: Extent| sectors > current.sectors)
            {
                best = Some(Extent::new(start, sectors));
            }
        }

        let best = best.ok_or(Error::Full)?;
        let sectors = best.sectors.min(remaining);
        plan.runs[plan.used] = Extent::new(best.start, sectors);
        plan.used += 1;
        remaining -= sectors;
    }
    Ok(plan)
}

/// Mark a previously planned set of free runs used. The caller plans and
/// reserves under one exclusive mutable borrow of the volume.
pub(super) fn reserve_plan(map: &mut [u64], plan: &PayloadPlan) {
    for run in plan.runs() {
        for sector in run.start..run.end() {
            set_allocated(map, sector, true);
        }
    }
}

pub(super) fn payload_crc(bytes: &[u8]) -> u32 {
    crc(bytes)
}

/// Stream borrowed bytes into the planned extents with one sector of scratch.
pub(super) fn write_payload(
    disk: &mut impl Disk,
    plan: &PayloadPlan,
    bytes: &[u8],
) -> Result<(), Error> {
    let mut offset = 0;
    let mut block = [0u8; 512];
    for run in plan.runs() {
        for sector in 0..run.sectors {
            block.fill(0);
            let count = (bytes.len() - offset).min(block.len());
            block[..count].copy_from_slice(&bytes[offset..offset + count]);
            disk.write(PAYLOAD_SECTOR + run.start + sector, &block)?;
            offset += count;
        }
    }
    if offset != bytes.len() {
        return Err(Error::Corrupt);
    }
    Ok(())
}

/// The payload-relative sector holding logical sector `index` of `runs`.
pub(super) fn run_sector(runs: &[Extent], mut index: u64) -> Option<u64> {
    for run in runs {
        if index < run.sectors {
            return Some(run.start + index);
        }
        index -= run.sectors;
    }
    None
}

pub(super) fn release_run(map: &mut [u64], run: Extent) -> Result<(), Error> {
    let mut free = crate::FreeSpace::new(map)?;
    free.release(run)
}

fn allocated(map: &[u64], sector: u64) -> bool {
    map[sector as usize / 64] & (1u64 << (sector % 64)) != 0
}

fn set_allocated(map: &mut [u64], sector: u64, used: bool) {
    let word = &mut map[sector as usize / 64];
    let bit = 1u64 << (sector % 64);
    if used {
        *word |= bit;
    } else {
        *word &= !bit;
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planner_uses_eight_runs_when_those_are_the_only_free_sectors() {
        let mut map = [u64::MAX; crate::extent::MAP_WORDS];
        for sector in (1..=15).step_by(2) {
            set_allocated(&mut map, sector, false);
        }

        let plan = plan_payload(&map, 8 * 512).unwrap();
        assert_eq!(plan.used, MAX_EXTENTS);
        assert!(plan.runs().iter().all(|run| run.sectors == 1));
    }
}
