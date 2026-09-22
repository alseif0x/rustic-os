// SPDX-License-Identifier: Apache-2.0
//! Mounting a v6 volume: the flush that makes a recovery trustworthy, the decode
//! that verifies every structure, and the clear that keeps a refused mount from
//! leaving a half-read table behind.

use super::{Volume6, header, map_checksum, nodes_checksum};
use crate::format6::{
    Header6, NODE_BYTES, Node6, RECEIPTS_SECTORS, map_sector, nodes_sector, receipts_sector,
};
use crate::receipt6::{BLOCK_BYTES, Receipts6};
use crate::{Disk, Error};

impl Volume6 {
    /// Mount into this value, decoding each structure straight into its field so
    /// no copy of the node table or the map is built on the stack.
    ///
    /// Mounting is the explicit recovery from an uncertain mount, so it first
    /// flushes the device and only then reads it: a failed commit may have left
    /// its header in a volatile cache, and adopting that state before it is
    /// durable would let a later commit overwrite the generation the device
    /// actually holds. The value is fenced until every checksum has been
    /// compared, and a refused mount clears the structures rather than leaving
    /// them half read.
    pub fn mount_into(&mut self, disk: &mut impl Disk) -> Result<(), Error> {
        self.fence();
        match self.trusted(disk) {
            Ok(()) => {
                self.unfence();
                Ok(())
            }
            Err(error) => {
                self.clear();
                Err(error)
            }
        }
    }
    /// Make what the device holds durable, then read and verify it. A device
    /// whose flush fails is not a recovery source.
    fn trusted(&mut self, disk: &mut impl Disk) -> Result<(), Error> {
        disk.flush()?;
        self.decode(disk)
    }
    /// Read every structure into the fields and verify it. The header is assigned
    /// last, so nothing is published until the checksums have agreed.
    fn decode(&mut self, disk: &mut impl Disk) -> Result<(), Error> {
        let header = header(disk)?;
        let nodes_base = nodes_sector(header.active);
        let mut sector = [0u8; 512];
        // One read per sector, four records per read: the guest pays a real block
        // round trip for each of these.
        for (chunk, window) in self.nodes.chunks_mut(512 / NODE_BYTES).enumerate() {
            disk.read(nodes_base + chunk as u64, &mut sector)?;
            for (offset, node) in window.iter_mut().enumerate() {
                let at = offset * NODE_BYTES;
                let mut record = [0u8; NODE_BYTES];
                record.copy_from_slice(&sector[at..at + NODE_BYTES]);
                *node = Node6::decode(&record)?;
            }
        }
        let map_base = map_sector(header.active);
        for (index, word) in self.map.iter_mut().enumerate() {
            if index % 64 == 0 {
                disk.read(map_base + (index / 64) as u64, &mut sector)?;
            }
            let at = (index % 64) * 8;
            *word = u64::from_le_bytes(sector[at..at + 8].try_into().unwrap());
        }
        let receipts_base = receipts_sector(header.active);
        let mut block = [0u8; BLOCK_BYTES];
        for index in 0..RECEIPTS_SECTORS {
            disk.read(receipts_base + index, &mut sector)?;
            block[index as usize * 512..(index as usize + 1) * 512].copy_from_slice(&sector);
        }
        let receipts = Receipts6::decode_block(&block)?;
        // The header commits to all three structures, so a corrupt table, map or
        // receipt block is caught here rather than during a later read.
        if map_checksum(&self.map) != header.map_checksum
            || nodes_checksum(&self.nodes) != header.nodes_checksum
            || receipts.checksum() != header.receipts_checksum
        {
            return Err(Error::Corrupt);
        }
        self.header = header;
        self.receipts = receipts;
        Ok(())
    }
    /// Drop everything a refused mount read, so no caller can observe half of a
    /// volume through [`Volume6::node`] or [`Volume6::free_sectors`]. The value
    /// stays fenced.
    fn clear(&mut self) {
        self.header = Header6::initial();
        self.nodes.fill(Node6::EMPTY);
        self.map.fill(0);
        self.receipts = Receipts6::EMPTY;
    }
}
