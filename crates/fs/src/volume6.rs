// SPDX-License-Identifier: Apache-2.0
//! Mounting, provisioning and payload I/O for a v6 volume (#51).
//!
//! The control records and the region geometry live in `format6`; the free-space
//! accounting lives in `extent`. This module is the owner that ties them to a
//! disk: it writes and verifies the header, the node table and the map, and it
//! moves file bytes through allocated extents. Copy-on-write publication and the
//! operation receipts stay with the v5 implementation until they are ported;
//! this layer is what makes the new layout mountable and testable on its own.

use crate::checksum::crc_update;
use crate::extent::{Extent, Extents, FreeSpace, MAP_WORDS, MAX_FILE_V6};
use crate::format6::{
    GENERATIONS, HEADER_SECTOR, Header6, NODE_BYTES, Node6, PAYLOAD_SECTOR, RECEIPTS_SECTORS,
    SECTOR_BYTES, map_sector, nodes_sector, receipts_sector,
};
use crate::receipt6::{BLOCK_BYTES, Receipt6, Receipts6};
use crate::{Disk, Error, Kind, OBJECTS_V6};

/// A mounted v6 volume: the verified header, the node table and the free-space
/// map, all owned in one place. The payload stays on the disk.
pub struct Volume6 {
    pub header: Header6,
    pub nodes: [Node6; OBJECTS_V6],
    pub(crate) map: [u64; MAP_WORDS],
    pub receipts: Receipts6,
}

/// The roots and the empty structures a fresh v6 volume starts from, built in
/// memory so a caller that must publish later (the v5 upgrade) can stage records
/// without touching the disk first.
pub(crate) fn blank(lineage: [u8; 16]) -> Result<Volume6, Error> {
    let mut volume = Volume6 {
        header: Header6::initial(),
        nodes: [Node6::EMPTY; OBJECTS_V6],
        map: [0; MAP_WORDS],
        receipts: Receipts6::new(lineage)?,
    };
    for (index, name) in [b"system".as_slice(), b"data", b"config", b"workspaces"]
        .iter()
        .enumerate()
    {
        let mut record = Node6::EMPTY;
        record.id = index as u32 + 1;
        record.parent = 0;
        record.version = index as u64 + 1;
        record.kind = Kind::Directory;
        record.name[..name.len()].copy_from_slice(name);
        record.name_length = name.len() as u8;
        volume.nodes[index] = record;
    }
    Ok(volume)
}

/// Write a fresh v6 volume: the four root directories, an empty map and a header
/// that checksums both. The caller owns the disk and its durability.
pub fn provision(disk: &mut impl Disk, lineage: [u8; 16]) -> Result<Volume6, Error> {
    let mut volume = blank(lineage)?;
    volume.flush(disk)?;
    Ok(volume)
}

/// Read and verify a v6 volume. Every structure is checksummed before it is
/// trusted, and a half-written volume is an error rather than a silent mount.
pub fn mount(disk: &mut impl Disk) -> Result<Volume6, Error> {
    let mut header_bytes = [0u8; SECTOR_BYTES as usize];
    disk.read(HEADER_SECTOR, &mut header_bytes)?;
    let header = Header6::decode(&header_bytes)?;

    let nodes_base = nodes_sector(header.active);
    let map_base = map_sector(header.active);
    let mut node_bytes = [0u8; 512];
    let mut nodes = [Node6::EMPTY; OBJECTS_V6];
    for (index, node) in nodes.iter_mut().enumerate() {
        let sector = nodes_base + (index * NODE_BYTES / 512) as u64;
        let offset = index * NODE_BYTES % 512;
        disk.read(sector, &mut node_bytes)?;
        let mut record = [0u8; NODE_BYTES];
        record.copy_from_slice(&node_bytes[offset..offset + NODE_BYTES]);
        *node = Node6::decode(&record)?;
    }

    let mut map = [0u64; MAP_WORDS];
    let mut sector = [0u8; 512];
    for (index, word) in map.iter_mut().enumerate() {
        if index % 64 == 0 {
            let at = map_base + (index / 64) as u64;
            disk.read(at, &mut sector)?;
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
    if map_checksum(&map) != header.map_checksum
        || nodes_checksum(&nodes) != header.nodes_checksum
        || receipts.checksum() != header.receipts_checksum
    {
        return Err(Error::Corrupt);
    }
    Ok(Volume6 {
        header,
        nodes,
        map,
        receipts,
    })
}

impl Volume6 {
    /// Retain a receipt and publish it with the next commit.
    pub fn retain_receipt(&mut self, disk: &mut impl Disk, receipt: Receipt6) -> Result<(), Error> {
        self.receipts.retain(receipt)?;
        self.flush(disk)
    }
    pub fn find_receipt(&self, retry: crate::recovery::Retry) -> Result<Option<&Receipt6>, Error> {
        self.receipts.find(retry)
    }
    pub fn node(&self, id: u32) -> Option<&Node6> {
        self.nodes
            .iter()
            .find(|node| node.id == id && node.kind != Kind::Empty)
    }
    pub fn free_sectors(&self) -> u64 {
        self.map
            .iter()
            .map(|word| u64::from(word.count_zeros()))
            .sum()
    }
    /// Persist the node table, the map and the header, in that order, so a torn
    /// write leaves the old checksums describing the old structures.
    pub fn flush(&mut self, disk: &mut impl Disk) -> Result<(), Error> {
        // Commit discipline: build the inactive generation completely, then let
        // one header sector publish it. Before that write the mounted generation
        // is still the active one, so a torn commit cannot be mounted as truth.
        let next = (self.header.active + 1) % GENERATIONS;
        let nodes_base = nodes_sector(next);
        let map_base = map_sector(next);
        for (index, node) in self.nodes.iter().enumerate() {
            let sector = nodes_base + (index * NODE_BYTES / 512) as u64;
            let offset = index * NODE_BYTES % 512;
            let mut block = [0u8; 512];
            disk.read(sector, &mut block)?;
            block[offset..offset + NODE_BYTES].copy_from_slice(&node.encode());
            disk.write(sector, &block)?;
        }
        for (index, chunk) in self.map.as_chunks::<64>().0.iter().enumerate() {
            let mut sector = [0u8; 512];
            for (word, value) in chunk.iter().enumerate() {
                sector[word * 8..word * 8 + 8].copy_from_slice(&value.to_le_bytes());
            }
            disk.write(map_base + index as u64, &sector)?;
        }
        let block = self.receipts.encode_block();
        for index in 0..RECEIPTS_SECTORS {
            let mut sector = [0u8; 512];
            sector.copy_from_slice(&block[index as usize * 512..(index as usize + 1) * 512]);
            disk.write(receipts_sector(next) + index, &sector)?;
        }
        self.header.nodes_checksum = nodes_checksum(&self.nodes);
        self.header.map_checksum = map_checksum(&self.map);
        self.header.receipts_checksum = self.receipts.checksum();
        self.header.active = next;
        self.header.sequence = self.header.sequence.saturating_add(1);
        disk.write(HEADER_SECTOR, &self.header.encode())?;
        disk.flush()
    }
    /// Write `bytes` into a node and record the extents that hold them, refusing
    /// a node whose current version is not `expected` so a stale writer cannot
    /// overwrite a newer one. The payload is allocated before anything is
    /// written, so a full or conflicting request leaves the node untouched.
    pub fn write_file(
        &mut self,
        disk: &mut impl Disk,
        index: usize,
        expected: u64,
        bytes: &[u8],
    ) -> Result<u64, Error> {
        if index >= OBJECTS_V6 || bytes.len() > MAX_FILE_V6 {
            return Err(Error::Size);
        }
        if self.nodes[index].kind == Kind::Empty {
            return Err(Error::NotFound);
        }
        if self.nodes[index].version != expected {
            return Err(Error::Version);
        }
        self.stage_bytes(disk, index, bytes)?;
        let node = &mut self.nodes[index];
        node.version = node.version.saturating_add(1);
        let version = node.version;
        self.flush(disk)?;
        Ok(version)
    }
    /// Allocate extents for `bytes`, write the payload and record it on the node.
    /// The version and the commit stay with the caller so a migration can keep
    /// the version it read; a failure before the payload is written leaves the
    /// node untouched, and only payload sectors are written until `flush`.
    pub(crate) fn stage_bytes(
        &mut self,
        disk: &mut impl Disk,
        index: usize,
        bytes: &[u8],
    ) -> Result<(), Error> {
        if index >= OBJECTS_V6 || bytes.len() > MAX_FILE_V6 {
            return Err(Error::Size);
        }
        if self.nodes[index].kind == Kind::Empty {
            return Err(Error::NotFound);
        }
        let sectors = (bytes.len() as u64).div_ceil(SECTOR_BYTES);
        let mut plan = Extents::new();
        let mut allocated: [Extent; 8] = [Extent::new(0, 0); 8];
        let mut used = 0;
        let mut remaining = sectors;
        while remaining > 0 {
            let want = remaining.min(64);
            let run = {
                let mut space = FreeSpace::new(&mut self.map).expect("map size is fixed");
                space.allocate(want)?
            };
            plan.push(run)?;
            allocated[used] = run;
            used += 1;
            remaining -= run.sectors;
        }
        // Payload first, then the record that points at it.
        let mut written = 0usize;
        for run in allocated[..used].iter() {
            for sector in 0..run.sectors {
                let mut block = [0u8; 512];
                let take = (bytes.len() - written).min(512);
                block[..take].copy_from_slice(&bytes[written..written + take]);
                written += take;
                disk.write(PAYLOAD_SECTOR + run.start + sector, &block)?;
            }
        }
        let node = &mut self.nodes[index];
        node.length = bytes.len() as u32;
        node.extents = [Extent::new(0, 0); 8];
        for (slot, run) in allocated[..used].iter().enumerate() {
            node.extents[slot] = *run;
        }
        node.extents_used = used as u8;
        Ok(())
    }
    /// Read a node's bytes into `out`, returning how many were copied. A record
    /// whose extents cannot hold its length is refused before any read.
    pub fn read_file(
        &self,
        disk: &mut impl Disk,
        node: &Node6,
        out: &mut [u8],
    ) -> Result<usize, Error> {
        let length = node.length as usize;
        if out.len() < length {
            return Err(Error::Size);
        }
        let capacity: u64 = node.runs().iter().map(|run| run.sectors).sum();
        if u64::from(node.length) > capacity * SECTOR_BYTES {
            return Err(Error::Corrupt);
        }
        let mut written = 0usize;
        for run in node.runs() {
            for sector in 0..run.sectors {
                let mut block = [0u8; 512];
                disk.read(PAYLOAD_SECTOR + run.start + sector, &mut block)?;
                let take = (length - written).min(512);
                out[written..written + take].copy_from_slice(&block[..take]);
                written += take;
                if written == length {
                    return Ok(written);
                }
            }
        }
        Ok(written)
    }
    /// Return a node's payload to the free map and empty its record.
    pub fn remove_file(&mut self, disk: &mut impl Disk, index: usize) -> Result<(), Error> {
        let runs = self.nodes[index].extents;
        let used = self.nodes[index].extents_used as usize;
        for run in runs[..used].iter() {
            let mut space = FreeSpace::new(&mut self.map).expect("map size is fixed");
            space.release(*run)?;
        }
        self.nodes[index] = Node6::EMPTY;
        self.flush(disk)
    }
}

fn nodes_checksum(nodes: &[Node6; OBJECTS_V6]) -> u32 {
    let mut state = !0u32;
    for node in nodes {
        crc_update(&mut state, &node.encode());
    }
    !state
}

fn map_checksum(map: &[u64; MAP_WORDS]) -> u32 {
    let mut state = !0u32;
    for word in map {
        crc_update(&mut state, &word.to_le_bytes());
    }
    !state
}
