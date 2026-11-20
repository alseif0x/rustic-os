// SPDX-License-Identifier: Apache-2.0
//! Ordinary version-checked replacement without durable retry records.
//!
//! This is the storage mechanism for the existing BEGIN/CHUNK/COMMIT contract.
//! The service owns the bounded candidate and authorization. Publication uses
//! the same copy-on-write payload and dual-generation barriers as tracked
//! replacement; retained snapshots and open-stage reservations remain owned.

use crate::format7::{MAX_FILE_BYTES, Node7, PAYLOAD_SECTOR};
use crate::{Disk, Error, Kind};

use super::Volume7;
use super::payload::{payload_crc, plan_payload, release_run, reserve_plan, run_sector};
use super::replacement::retained_owns_run;

impl Volume7 {
    /// Replace a live writable file under an exact version precondition.
    ///
    /// No retry identity or receipt is allocated: a lost response remains
    /// uncertain to the caller, which must inspect the file rather than replay
    /// it as a tracked operation. Even a full retained-record table permits
    /// ordinary replacement when copy-on-write payload capacity is available.
    /// Argument, version and capacity refusals occur before I/O. Any failure
    /// after payload writes start fences the owner and returns `Uncertain`;
    /// remount chooses the old or new complete generation.
    pub fn replace(
        &mut self,
        disk: &mut impl Disk,
        id: u32,
        expected_version: u64,
        bytes: &[u8],
    ) -> Result<Node7, Error> {
        self.ready()?;
        if bytes.len() > MAX_FILE_BYTES as usize {
            return Err(Error::Size);
        }
        let index = self
            .nodes
            .iter()
            .position(|node| node.kind != Kind::Empty && node.id == id)
            .ok_or(Error::NotFound)?;
        let previous = self.nodes[index];
        if previous.space == 1 {
            return Err(Error::ReadOnly);
        }
        if previous.kind != Kind::File {
            return Err(Error::IsDirectory);
        }
        if previous.version != expected_version {
            return Err(Error::Version);
        }
        let version = self
            .header
            .sequence
            .checked_add(1)
            .ok_or(Error::Exhausted)?;
        if version <= expected_version {
            return Err(Error::Corrupt);
        }
        // Open stages reserve sectors outside the durable bitmap. Include
        // their overlay when selecting our own runs, without publishing it.
        let plan = plan_payload(self.planning_map(), bytes.len())?;
        let next = Node7 {
            version,
            length: bytes.len() as u32,
            payload_crc32: payload_crc(bytes),
            extents_used: plan.used as u8,
            extents: plan.runs,
            ..previous
        };
        next.encode()?;

        self.fenced = true;
        let result = (|| {
            for (index, chunk) in bytes.chunks(512).enumerate() {
                let sector = run_sector(plan.runs(), index as u64).ok_or(Error::Corrupt)?;
                let mut block = [0; 512];
                block[..chunk.len()].copy_from_slice(chunk);
                disk.write(PAYLOAD_SECTOR + sector, &block)?;
            }
            reserve_plan(&mut self.map, &plan);
            for run in previous.runs() {
                if !retained_owns_run(&self.records, *run) {
                    release_run(&mut self.map, *run)?;
                }
            }
            self.nodes[index] = next;
            self.publish_candidate(disk)
        })();
        match result {
            Ok(header) => {
                self.header = header;
                self.fenced = false;
                Ok(next)
            }
            Err(_) => {
                self.fence_clear();
                Err(Error::Uncertain)
            }
        }
    }
}
