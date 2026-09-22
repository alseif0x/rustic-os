// SPDX-License-Identifier: Apache-2.0
//! Mounting, provisioning and payload I/O for a v6 volume (#51).
//!
//! The control records and the region geometry live in `format6`; the free-space
//! accounting lives in `extent`. This module is the owner that ties them to a
//! disk: it writes and verifies the header, the node table and the map, and it
//! moves file bytes through allocated extents. The production file service still
//! uses v5; this layer provides standalone v6 publication and bounded receipts.
//!
//! # Commit discipline
//!
//! One commit reserves payload, writes it, builds the inactive generation, makes
//! it durable, and only then lets one header sector publish it:
//!
//! 1. the free-space map reserves the runs and the payload is written into those
//!    sectors;
//! 2. the node table, the map and the receipt block are written into the inactive
//!    generation;
//! 3. a successful flush makes steps 1 and 2 durable;
//! 4. the header sector naming the inactive generation is written;
//! 5. a second successful flush makes the publication durable, and only then is
//!    that header adopted in memory.
//!
//! Steps 1 and 2 touch nothing the mounted header names, and step 4 cannot run
//! before step 3 succeeded, so a failure at any point leaves the generation the
//! header names intact and mountable, as long as the device applies a sector
//! atomically or reports a failed write before it lands. A failure after the
//! first sector write can still leave part of the commit on the device, so the
//! mount is *fenced*: mutations, payload reads and receipt lookup/replay answer
//! [`Error::Uncertain`] until the caller mounts again. Public fields, `node` and
//! `free_sectors` remain low-level inspection, not durability evidence.
//! Mounting is the explicit recovery, and it flushes the device before reading
//! anything from it: reading a write cache back is not evidence that a state
//! became durable.
//!
//! The layout keeps one header sector and no second copy, so a header torn
//! between two commits is a `Corrupt` mount rather than a fallback. The
//! discipline above assumes atomic sector writes; the `Disk` trait does not
//! guarantee them, and not every medium always leaves a mountable volume.
//!
//! Validation refusals that happen before the first write (`Size`, `NotFound`,
//! `Version`, `Full`, `IdempotencyConflict`, `ExpiredEpoch`, `Lineage`) do not
//! fence the mount and reserve nothing.

use crate::checksum::crc_update;
use crate::extent::{FreeSpace, MAP_WORDS, MAX_FILE_V6};
use crate::format6::{HEADER_SECTOR, Header6, Node6, PAYLOAD_SECTOR, SECTOR_BYTES};
use crate::receipt6::{RETAINED_V6, Receipt6, Receipts6};
use crate::{Disk, Error, Kind, OBJECTS_V6};

/// The mechanisms behind the public entry points: mounting and verification,
/// payload staging, and the publication transaction.
mod mount;
mod payload;
mod publication;

/// A mounted v6 volume: the verified header, the node table and the free-space
/// map, all owned in one place. The payload stays on the disk.
pub struct Volume6 {
    pub header: Header6,
    pub nodes: [Node6; OBJECTS_V6],
    pub(crate) map: [u64; MAP_WORDS],
    pub receipts: Receipts6,
    /// Set while the mounted state may no longer describe the device, because an
    /// operation wrote part of a commit. Nothing is written or answered until a
    /// mount succeeds again.
    poisoned: bool,
}

impl Volume6 {
    /// An unmounted volume. The node table is larger than a process stack, so a
    /// caller that has a static or a heap buffer mounts in place through
    /// [`Volume6::mount_into`] instead of moving the whole table by value.
    pub const EMPTY: Self = Self {
        header: Header6::initial(),
        nodes: [Node6::EMPTY; OBJECTS_V6],
        map: [0; MAP_WORDS],
        receipts: Receipts6::EMPTY,
        poisoned: false,
    };
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
        poisoned: false,
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
    let mut volume = Volume6::EMPTY;
    volume.mount_into(disk)?;
    Ok(volume)
}

/// The header sector, checked on its own so a caller can decide about the rest.
fn header(disk: &mut impl Disk) -> Result<Header6, Error> {
    let mut bytes = [0u8; SECTOR_BYTES as usize];
    disk.read(HEADER_SECTOR, &mut bytes)?;
    Header6::decode(&bytes)
}

impl Volume6 {
    /// Retain a receipt and publish it with the next commit.
    pub fn retain_receipt(&mut self, disk: &mut impl Disk, receipt: Receipt6) -> Result<(), Error> {
        self.ready()?;
        // Validation first: a foreign lineage, a stale epoch or a full table is
        // the caller's condition, not an uncertain mount.
        self.receipts.retain(receipt)?;
        // The table now holds a receipt that is not published yet: nothing may be
        // answered from it until the commit settles.
        self.fence();
        self.publish(disk)?;
        self.unfence();
        Ok(())
    }
    /// The retained record for a retry, or `None`. A fenced mount answers
    /// `Uncertain`: its table may hold a receipt whose commit never settled.
    pub fn find_receipt(&self, retry: crate::recovery::Retry) -> Result<Option<&Receipt6>, Error> {
        self.ready()?;
        self.receipts.find(retry)
    }
    pub fn node(&self, id: u32) -> Option<&Node6> {
        self.nodes
            .iter()
            .find(|node| node.id == id && node.kind != Kind::Empty)
    }
    /// The free sectors the in-memory map holds. It is an accounting view of the
    /// mounted volume, not a durability claim.
    pub fn free_sectors(&self) -> u64 {
        self.map
            .iter()
            .map(|word| u64::from(word.count_zeros()))
            .sum()
    }
    /// Publish the current in-memory structures: build the inactive generation,
    /// make it durable, then let one header sector name it.
    pub fn flush(&mut self, disk: &mut impl Disk) -> Result<(), Error> {
        self.ready()?;
        self.publish(disk)?;
        self.unfence();
        Ok(())
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
        self.ready()?;
        if index >= OBJECTS_V6 || bytes.len() > MAX_FILE_V6 {
            return Err(Error::Size);
        }
        if self.nodes[index].kind == Kind::Empty {
            return Err(Error::NotFound);
        }
        if self.nodes[index].version != expected {
            return Err(Error::Version);
        }
        if let Err(error) = self.stage(disk, index, bytes) {
            return Err(self.outcome(error));
        }
        // The staged state is not published yet: nothing may be answered until
        // the version, the map and the payload are one durable generation.
        self.fence();
        let node = &mut self.nodes[index];
        node.version = node.version.saturating_add(1);
        let version = node.version;
        self.publish(disk)?;
        self.unfence();
        Ok(version)
    }
    /// Write `bytes` and retain the operation identity that produced them, so
    /// data, version and receipt are published by one commit. Repeating a retry
    /// returns the retained receipt without writing; a retry that describes a
    /// different record is a conflict, and a full table is `Full` before any
    /// payload is staged. Unlike the v5 record, a v6 receipt keeps no snapshot of
    /// the replaced bytes, so a replay matches identity, previous version and
    /// length rather than content.
    pub fn write_tracked(
        &mut self,
        disk: &mut impl Disk,
        index: usize,
        expected: u64,
        retry: crate::recovery::Retry,
        bytes: &[u8],
    ) -> Result<Receipt6, Error> {
        self.ready()?;
        if index >= OBJECTS_V6 || bytes.len() > MAX_FILE_V6 {
            return Err(Error::Size);
        }
        let id = self.nodes[index].id;
        if self.nodes[index].kind == Kind::Empty {
            return Err(Error::NotFound);
        }
        if let Some(record) = self.receipts.find(retry)? {
            if record.id != id || record.previous != expected || record.length != bytes.len() as u32
            {
                return Err(Error::IdempotencyConflict);
            }
            return Ok(*record);
        }
        if self.nodes[index].version != expected {
            return Err(Error::Version);
        }
        if self.receipts.len() >= RETAINED_V6 {
            return Err(Error::Full);
        }
        if let Err(error) = self.stage(disk, index, bytes) {
            return Err(self.outcome(error));
        }
        self.fence();
        let node = &mut self.nodes[index];
        node.version = node.version.saturating_add(1);
        let receipt = Receipt6 {
            retry,
            id,
            previous: expected,
            committed: node.version,
            length: bytes.len() as u32,
        };
        // The table was checked above, so a refusal here can only mean the
        // in-memory state no longer matches what was validated.
        if self.receipts.retain(receipt).is_err() {
            return Err(Error::Uncertain);
        }
        self.publish(disk)?;
        self.unfence();
        Ok(receipt)
    }
    /// Allocate extents for `bytes`, write the payload and record it on the node.
    /// The version and the commit stay with the caller so a migration can keep
    /// the version it read; a failure before the payload is written leaves the
    /// node untouched and the map unchanged, and only payload sectors are
    /// written until `flush`.
    pub(crate) fn stage_bytes(
        &mut self,
        disk: &mut impl Disk,
        index: usize,
        bytes: &[u8],
    ) -> Result<(), Error> {
        self.ready()?;
        if index >= OBJECTS_V6 || bytes.len() > MAX_FILE_V6 {
            return Err(Error::Size);
        }
        if self.nodes[index].kind == Kind::Empty {
            return Err(Error::NotFound);
        }
        // The migration owns a fresh target it discards on failure, so staging
        // reports what the disk said while the target is fenced against use.
        self.stage(disk, index, bytes)
    }
    /// Read a node's bytes into `out`, returning how many were copied. A record
    /// whose extents cannot hold its length is refused before any read.
    pub fn read_file(
        &self,
        disk: &mut impl Disk,
        node: &Node6,
        out: &mut [u8],
    ) -> Result<usize, Error> {
        self.ready()?;
        let length = node.length as usize;
        if out.len() < length {
            return Err(Error::Size);
        }
        self.read_range(disk, node, 0, &mut out[..length])
    }
    /// Read at most `out.len()` bytes from `offset`, streaming only the extents
    /// the range touches so a caller needs no buffer for the whole file. A range
    /// that starts past the end is `Size`; one that runs past the end copies what
    /// the file holds and returns that count.
    pub fn read_range(
        &self,
        disk: &mut impl Disk,
        node: &Node6,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, Error> {
        self.ready()?;
        let length = u64::from(node.length);
        if offset > length {
            return Err(Error::Size);
        }
        let held: u64 = node.runs().iter().map(|run| run.sectors).sum();
        if length > held * SECTOR_BYTES {
            return Err(Error::Corrupt);
        }
        let count = out.len().min((length - offset) as usize);
        let end = offset + count as u64;
        let mut position = 0u64;
        let mut written = 0usize;
        for run in node.runs() {
            let start = position;
            position += run.sectors * SECTOR_BYTES;
            if position <= offset {
                continue;
            }
            if start >= end {
                break;
            }
            let from = offset.max(start);
            let to = end.min(position);
            let mut sector = (from - start) / SECTOR_BYTES;
            let mut within = (from - start) % SECTOR_BYTES;
            let mut remaining = (to - from) as usize;
            while remaining > 0 {
                let mut block = [0u8; 512];
                disk.read(PAYLOAD_SECTOR + run.start + sector, &mut block)?;
                let take = remaining.min(512 - within as usize);
                out[written..written + take]
                    .copy_from_slice(&block[within as usize..within as usize + take]);
                written += take;
                remaining -= take;
                sector += 1;
                within = 0;
            }
        }
        Ok(written)
    }
    /// Remove a record and give its runs back.
    pub fn remove_file(&mut self, disk: &mut impl Disk, index: usize) -> Result<(), Error> {
        self.ready()?;
        if index >= OBJECTS_V6 {
            return Err(Error::Size);
        }
        if self.nodes[index].kind == Kind::Empty {
            return Err(Error::NotFound);
        }
        // Validation is done; the record and the accounting change from here, so
        // the mount is fenced until the removal is published.
        self.fence();
        let runs = self.nodes[index].extents;
        let used = self.nodes[index].extents_used as usize;
        for run in runs[..used].iter() {
            let mut space = FreeSpace::new(&mut self.map).expect("map size is fixed");
            if space.release(*run).is_err() {
                // The record and the map disagree: neither can be trusted.
                return Err(Error::Uncertain);
            }
        }
        self.nodes[index] = Node6::EMPTY;
        self.publish(disk)?;
        self.unfence();
        Ok(())
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
