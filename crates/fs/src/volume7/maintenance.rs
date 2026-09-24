// SPDX-License-Identifier: Apache-2.0
//! Explicit retry-epoch advancement and terminal-record reclamation for v7.

use crate::extent::DATA_SECTORS;
use crate::format7::{MAP_WORDS, NODES, Node7, RecordState, validate_generation};
use crate::{Disk, Error, Kind};

use super::Volume7;

impl Volume7 {
    /// Drop every terminal retained outcome, reclaiming only payload sectors no
    /// live file owns, and publish the next retry epoch. Returns that epoch.
    ///
    /// A service may call this only after clients have resolved every outcome
    /// from the current epoch. Storage cannot know whether a completed response
    /// was observed. Once published, retries from the old epoch return
    /// [`Error::ExpiredEpoch`]. This operation never runs implicitly to make
    /// room in a full receipt table. It returns [`Error::Busy`] while any
    /// streamed stage is open.
    pub fn maintain_retention(&mut self, disk: &mut impl Disk) -> Result<u64, Error> {
        self.ready()?;

        // An open stage depends on the current epoch, its retained retry
        // record or a reserved receipt slot.
        if self.stages_open() {
            return Err(Error::Busy);
        }

        // An unresolved admission may still complete or be cancelled, so it
        // must remain addressable until its outcome is terminal.
        if self
            .records
            .iter()
            .flatten()
            .any(|record| record.state == RecordState::Admitted)
        {
            return Err(Error::Busy);
        }

        let next_epoch = self.header.epoch.checked_add(1).ok_or(Error::Exhausted)?;
        self.header
            .sequence
            .checked_add(1)
            .ok_or(Error::Exhausted)?;

        // Validate current ownership before deriving the map that remains after
        // all terminal snapshots have been discarded.
        validate_generation(
            &self.header,
            &self.nodes,
            &self.records,
            &self.map,
            &mut self.validation,
        )?;
        live_file_map(&self.nodes, &mut self.validation)?;
        if self
            .validation
            .iter()
            .zip(self.map.iter())
            .any(|(live, current)| *live & !*current != 0)
        {
            return Err(Error::Corrupt);
        }

        self.fenced = true;
        self.records.fill(None);
        self.map.copy_from_slice(&self.validation);
        self.header.epoch = next_epoch;

        match self.publish_candidate(disk) {
            Ok(header) => {
                self.header = header;
                self.fenced = false;
                Ok(next_epoch)
            }
            Err(error) => {
                self.clear();
                self.fenced = true;
                Err(error)
            }
        }
    }
}

fn live_file_map(nodes: &[Node7; NODES], map: &mut [u64; MAP_WORDS]) -> Result<(), Error> {
    map.fill(0);
    for node in nodes.iter().filter(|node| node.kind == Kind::File) {
        for run in node.runs() {
            let end = run.start.checked_add(run.sectors).ok_or(Error::Corrupt)?;
            if run.sectors == 0 || end > DATA_SECTORS {
                return Err(Error::Corrupt);
            }
            for sector in run.start..end {
                let word = &mut map[(sector / 64) as usize];
                let bit = 1u64 << (sector % 64);
                if *word & bit != 0 {
                    return Err(Error::Corrupt);
                }
                *word |= bit;
            }
        }
    }
    Ok(())
}
