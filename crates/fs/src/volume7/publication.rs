// SPDX-License-Identifier: Apache-2.0
//! Dual-generation v7 publication for a fully staged direct commit.

use crate::checksum::crc_update;
use crate::format7::{
    Header7, MAP_SECTORS, NODE_BYTES, NODES_SECTORS, RECEIPTS_SECTORS, RECORD_BYTES, header_sector,
    map_sector, nodes_sector, receipts_sector, validate_generation,
};
use crate::{Disk, Error};

use super::Volume7;

impl Volume7 {
    /// Persist the candidate structures to the inactive generation, then name
    /// them with its header. The caller keeps the owner fenced until adoption.
    pub(super) fn publish_candidate(&mut self, disk: &mut impl Disk) -> Result<Header7, Error> {
        let candidate = Header7 {
            sequence: self
                .header
                .sequence
                .checked_add(1)
                .ok_or(Error::Exhausted)?,
            generation: self.header.generation ^ 1,
            ..self.header
        };
        validate_generation(
            &candidate,
            &self.nodes,
            &self.records,
            &self.map,
            &mut self.validation,
        )?;

        let (nodes_checksum, map_checksum, receipts_checksum) =
            self.write_generation(disk, candidate.generation)?;
        let published = Header7 {
            nodes_checksum,
            map_checksum,
            receipts_checksum,
            ..candidate
        };
        let encoded = published.encode()?;

        // Payload and inactive metadata must be durable before either header
        // can select the candidate generation.
        disk.flush().map_err(|_| Error::Uncertain)?;
        disk.write(header_sector(published.generation), &encoded)
            .map_err(|_| Error::Uncertain)?;
        disk.flush().map_err(|_| Error::Uncertain)?;
        Ok(published)
    }

    fn write_generation(
        &self,
        disk: &mut impl Disk,
        generation: u8,
    ) -> Result<(u32, u32, u32), Error> {
        let mut block = [0u8; 512];
        let mut nodes_checksum = !0u32;
        let base = nodes_sector(generation);
        for sector in 0..NODES_SECTORS {
            block.fill(0);
            for offset in 0..512 / NODE_BYTES {
                let index = sector as usize * (512 / NODE_BYTES) + offset;
                let encoded = self.nodes[index].encode()?;
                let start = offset * NODE_BYTES;
                block[start..start + NODE_BYTES].copy_from_slice(&encoded);
            }
            crc_update(&mut nodes_checksum, &block);
            disk.write(base + sector, &block)
                .map_err(|_| Error::Uncertain)?;
        }

        let mut map_checksum = !0u32;
        let base = map_sector(generation);
        for sector in 0..MAP_SECTORS {
            for offset in 0..512 / 8 {
                let index = sector as usize * (512 / 8) + offset;
                block[offset * 8..offset * 8 + 8].copy_from_slice(&self.map[index].to_le_bytes());
            }
            crc_update(&mut map_checksum, &block);
            disk.write(base + sector, &block)
                .map_err(|_| Error::Uncertain)?;
        }

        let mut receipts_checksum = !0u32;
        let base = receipts_sector(generation);
        for sector in 0..RECEIPTS_SECTORS {
            block.fill(0);
            let block_start = sector as usize * 512;
            let block_end = block_start + 512;
            for (index, record) in self.records.iter().enumerate() {
                let Some(record) = record else {
                    continue;
                };
                let record_start = index * RECORD_BYTES;
                let record_end = record_start + RECORD_BYTES;
                let copy_start = block_start.max(record_start);
                let copy_end = block_end.min(record_end);
                if copy_start >= copy_end {
                    continue;
                }
                let encoded = record.encode()?;
                let source = copy_start - record_start;
                let destination = copy_start - block_start;
                let count = copy_end - copy_start;
                block[destination..destination + count]
                    .copy_from_slice(&encoded[source..source + count]);
            }
            crc_update(&mut receipts_checksum, &block);
            disk.write(base + sector, &block)
                .map_err(|_| Error::Uncertain)?;
        }
        Ok((!nodes_checksum, !map_checksum, !receipts_checksum))
    }
}
