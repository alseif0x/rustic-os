// SPDX-License-Identifier: Apache-2.0
//! The payload half of a staged v6 write: the free-space reservation, the sector
//! writes, and the record that points at them.

use super::Volume6;
use crate::extent::{EXTENTS_PER_FILE, Extent, FILE_SECTORS_MAX, FreeSpace};
use crate::format6::{PAYLOAD_SECTOR, SECTOR_BYTES};
use crate::{Disk, Error};

/// The runs one staged write occupies, in the order its bytes fill them. Its size
/// is the record's, so a plan is as bounded as a node's extent list.
#[derive(Clone, Copy)]
struct Plan {
    runs: [Extent; EXTENTS_PER_FILE],
    used: usize,
}

impl Plan {
    const EMPTY: Self = Self {
        runs: [Extent::new(0, 0); EXTENTS_PER_FILE],
        used: 0,
    };
    fn runs(&self) -> &[Extent] {
        &self.runs[..self.used]
    }
}

impl Volume6 {
    /// Reserve the runs `bytes` need, write them and move the record onto them.
    /// A refusal before the first write leaves the map as it was and the mount
    /// usable; any failure from the payload write on fences the mount, because
    /// the device may have kept part of the payload.
    pub(super) fn stage(
        &mut self,
        disk: &mut impl Disk,
        index: usize,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let plan = self.reserve(bytes.len())?;
        if let Err(error) = self.write_payload(disk, &plan, bytes) {
            self.fence();
            return Err(error);
        }
        if let Err(error) = self.adopt(index, &plan, bytes.len()) {
            self.fence();
            return Err(error);
        }
        Ok(())
    }
    /// Allocate the runs `length` bytes need. Allocation is accounting, not I/O:
    /// a refusal (`Size` or `Full`) gives back everything it reserved before it
    /// failed, so a refused plan leaves the map exactly as it was.
    fn reserve(&mut self, length: usize) -> Result<Plan, Error> {
        let sectors = (length as u64).div_ceil(SECTOR_BYTES);
        if sectors > FILE_SECTORS_MAX {
            return Err(Error::Size);
        }
        let mut plan = Plan::EMPTY;
        let mut remaining = sectors;
        while remaining > 0 {
            let want = remaining.min(64);
            let allocated = {
                let mut space = FreeSpace::new(&mut self.map).expect("map size is fixed");
                space.allocate(want)
            };
            let run = match allocated {
                Ok(run) => run,
                Err(error) => {
                    self.rollback(&plan)?;
                    return Err(error);
                }
            };
            if plan.used == EXTENTS_PER_FILE {
                self.rollback(&plan)?;
                return Err(Error::Full);
            }
            plan.runs[plan.used] = run;
            plan.used += 1;
            remaining -= run.sectors;
        }
        Ok(plan)
    }
    /// Give back what a refused plan reserved. A release that does not match the
    /// map means the accounting itself is broken, so the mount is fenced.
    fn rollback(&mut self, plan: &Plan) -> Result<(), Error> {
        for run in plan.runs() {
            let mut space = FreeSpace::new(&mut self.map).expect("map size is fixed");
            if space.release(*run).is_err() {
                self.fence();
                return Err(Error::Uncertain);
            }
        }
        Ok(())
    }
    /// Write the payload into the reserved runs. Only payload sectors are
    /// touched, so a failure here leaves the published generation intact while
    /// the caller decides whether the operation may still have happened.
    fn write_payload(
        &mut self,
        disk: &mut impl Disk,
        plan: &Plan,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let mut written = 0usize;
        for run in plan.runs() {
            for sector in 0..run.sectors {
                let mut block = [0u8; 512];
                let take = (bytes.len() - written).min(512);
                block[..take].copy_from_slice(&bytes[written..written + take]);
                written += take;
                disk.write(PAYLOAD_SECTOR + run.start + sector, &block)?;
            }
        }
        Ok(())
    }
    /// Move the record onto the new runs and give the runs it replaced back. The
    /// release happens after the payload exists: releasing first would let the
    /// new payload overwrite the bytes the previous version still points at, and
    /// keeping the old allocation would leak it until the volume filled.
    fn adopt(&mut self, index: usize, plan: &Plan, length: usize) -> Result<(), Error> {
        let previous = self.nodes[index].extents;
        let previous_used = self.nodes[index].extents_used as usize;
        for run in previous[..previous_used].iter() {
            let mut space = FreeSpace::new(&mut self.map).expect("map size is fixed");
            if space.release(*run).is_err() {
                return Err(Error::Uncertain);
            }
        }
        let node = &mut self.nodes[index];
        node.length = length as u32;
        node.extents = [Extent::new(0, 0); EXTENTS_PER_FILE];
        for (slot, run) in plan.runs().iter().enumerate() {
            node.extents[slot] = *run;
        }
        node.extents_used = plan.used as u8;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extent::{DATA_SECTORS, MAP_WORDS, MAX_FILE_V6};

    /// A map whose only free sectors are the first `free` of the payload region,
    /// so a plan larger than that has to be refused part way through.
    fn free_to(free: u64) -> [u64; MAP_WORDS] {
        let mut map = [u64::MAX; MAP_WORDS];
        for sector in 0..free {
            map[sector as usize / 64] &= !(1 << (sector % 64));
        }
        map
    }

    #[test]
    fn a_refused_plan_leaves_the_free_space_map_unchanged() {
        // 100 free sectors cannot hold the 512 sectors a 256 KiB write needs: the
        // first 64-sector reservation succeeds, the second must give it back.
        let mut volume = Volume6::EMPTY;
        volume.map = free_to(100);
        let before = volume.map;
        assert_eq!(volume.reserve(MAX_FILE_V6).err(), Some(Error::Full));
        assert_eq!(volume.map, before, "a refused plan must not leak");
        assert_eq!(volume.free_sectors(), 100);
        assert!(!volume.poisoned, "a refusal is not an uncertain state");
    }

    #[test]
    fn a_plan_beyond_the_per_file_extent_cap_is_refused_before_the_map_is_touched() {
        let mut volume = Volume6::EMPTY;
        volume.map = free_to(DATA_SECTORS);
        let before = volume.map;
        let bytes = (FILE_SECTORS_MAX as usize + 1) * SECTOR_BYTES as usize;
        assert_eq!(volume.reserve(bytes).err(), Some(Error::Size));
        assert_eq!(volume.map, before);
        assert!(!volume.poisoned);
    }
}
