// SPDX-License-Identifier: Apache-2.0
//! The publication half of the v6 commit discipline: fence, publish, unfence.
//!
//! A failed publication leaves the fence raised, so subsequent mutations and
//! receipt lookups answer `Uncertain` until a successful mount clears it.

use super::{Volume6, map_checksum, nodes_checksum};
use crate::format6::{
    GENERATIONS, HEADER_SECTOR, NODE_BYTES, RECEIPTS_SECTORS, map_sector, nodes_sector,
    receipts_sector,
};
use crate::{Disk, Error};

impl Volume6 {
    /// Refuse to touch a mount whose last operation left the device and the
    /// mounted state disagreeing.
    pub(super) fn ready(&self) -> Result<(), Error> {
        if self.poisoned {
            Err(Error::Uncertain)
        } else {
            Ok(())
        }
    }
    pub(super) fn fence(&mut self) {
        self.poisoned = true;
    }
    pub(super) fn unfence(&mut self) {
        self.poisoned = false;
    }
    /// The error a public operation reports: a refusal before the first write is
    /// the caller's own condition, and a failure after it is an unknown outcome.
    pub(super) fn outcome(&self, error: Error) -> Error {
        if self.poisoned {
            Error::Uncertain
        } else {
            error
        }
    }
    /// Publish, fencing the mount if any part of the publication fails.
    pub(super) fn publish(&mut self, disk: &mut impl Disk) -> Result<(), Error> {
        match self.write_generation(disk) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.fence();
                Err(error)
            }
        }
    }
    /// Write the inactive generation, flush it, and then let one header sector
    /// name it. The header is written only after a successful flush of the
    /// payload and the generation, and adopted in memory only after the final
    /// flush succeeded, so a device that persists the header early can never
    /// present a generation whose structures did not land.
    fn write_generation(&mut self, disk: &mut impl Disk) -> Result<(), Error> {
        let next = (self.header.active + 1) % GENERATIONS;
        let nodes_base = nodes_sector(next);
        let map_base = map_sector(next);
        for (index, node) in self.nodes.iter().enumerate() {
            let sector = nodes_base + (index * NODE_BYTES / 512) as u64;
            let offset = index * NODE_BYTES % 512;
            let mut block = [0u8; 512];
            disk.read(sector, &mut block)
                .map_err(|_| Error::Uncertain)?;
            block[offset..offset + NODE_BYTES].copy_from_slice(&node.encode());
            disk.write(sector, &block).map_err(|_| Error::Uncertain)?;
        }
        for (index, chunk) in self.map.as_chunks::<64>().0.iter().enumerate() {
            let mut sector = [0u8; 512];
            for (word, value) in chunk.iter().enumerate() {
                sector[word * 8..word * 8 + 8].copy_from_slice(&value.to_le_bytes());
            }
            disk.write(map_base + index as u64, &sector)
                .map_err(|_| Error::Uncertain)?;
        }
        let block = self.receipts.encode_block();
        for index in 0..RECEIPTS_SECTORS {
            let mut sector = [0u8; 512];
            sector.copy_from_slice(&block[index as usize * 512..(index as usize + 1) * 512]);
            disk.write(receipts_sector(next) + index, &sector)
                .map_err(|_| Error::Uncertain)?;
        }
        // The barrier: until this flush succeeds the header must not name the
        // generation above, however the device orders the sectors it caches.
        disk.flush().map_err(|_| Error::Uncertain)?;
        let mut published = self.header;
        published.nodes_checksum = nodes_checksum(&self.nodes);
        published.map_checksum = map_checksum(&self.map);
        published.receipts_checksum = self.receipts.checksum();
        published.active = next;
        published.sequence = self.header.sequence.saturating_add(1);
        disk.write(HEADER_SECTOR, &published.encode())
            .map_err(|_| Error::Uncertain)?;
        disk.flush().map_err(|_| Error::Uncertain)?;
        // The final flush is confirmed: the header this mount describes is the
        // one the device holds.
        self.header = published;
        Ok(())
    }
}
