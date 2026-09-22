// SPDX-License-Identifier: Apache-2.0
//! Header recovery and full read-only v7 generation verification.

use crate::checksum::crc_update;
use crate::format7::{
    Header7, MAP_SECTORS, NODE_BYTES, NODES_SECTORS, Node7, RECEIPT_BLOCK_BYTES, RECEIPTS_SECTORS,
    header_sector, map_sector, nodes_sector, receipt_slots, receipts_sector, validate_generation,
};
use crate::{Disk, Error};

use super::Volume7;
use super::payload::verify_payloads;

impl Volume7 {
    /// Flush, recover the best valid header pair, and load its named generation
    /// directly into this value. Any refusal clears all partial state and leaves
    /// the object fenced; only successful verification makes accessors usable.
    pub fn mount_into(&mut self, disk: &mut impl Disk) -> Result<(), Error> {
        self.fenced = true;
        self.clear();
        match self.mount_trusted(disk) {
            Ok(()) => {
                self.fenced = false;
                Ok(())
            }
            Err(error) => {
                self.clear();
                Err(error)
            }
        }
    }

    fn mount_trusted(&mut self, disk: &mut impl Disk) -> Result<(), Error> {
        disk.flush()?;
        let mut copies = [[0u8; 512]; 2];
        for (slot, bytes) in copies.iter_mut().enumerate() {
            disk.read(header_sector(slot as u8), bytes)?;
        }
        let (header, recovered) = select_header(&copies)?;
        self.read_generation(disk, &header)?;

        validate_generation(
            &header,
            &self.nodes,
            &self.records,
            &self.map,
            &mut self.validation,
        )?;
        verify_payloads(disk, &self.nodes, &self.records)?;

        self.header = header;
        self.recovered_from_header = recovered;
        Ok(())
    }

    fn read_generation(&mut self, disk: &mut impl Disk, header: &Header7) -> Result<(), Error> {
        let mut block = [0u8; 512];
        let mut nodes_crc = !0u32;
        let base = nodes_sector(header.generation);
        for sector in 0..NODES_SECTORS {
            disk.read(base + sector, &mut block)?;
            crc_update(&mut nodes_crc, &block);
            for offset in 0..512 / NODE_BYTES {
                let index = sector as usize * (512 / NODE_BYTES) + offset;
                let at = offset * NODE_BYTES;
                let encoded: &[u8; NODE_BYTES] = block[at..at + NODE_BYTES]
                    .try_into()
                    .map_err(|_| Error::Corrupt)?;
                self.nodes[index] = Node7::decode(encoded)?;
            }
        }
        if !nodes_crc != header.nodes_checksum {
            return Err(Error::Corrupt);
        }

        let mut map_crc = !0u32;
        let base = map_sector(header.generation);
        for sector in 0..MAP_SECTORS {
            disk.read(base + sector, &mut block)?;
            crc_update(&mut map_crc, &block);
            for offset in 0..512 / 8 {
                let index = sector as usize * (512 / 8) + offset;
                let at = offset * 8;
                self.map[index] = u64::from_le_bytes(block[at..at + 8].try_into().unwrap());
            }
        }
        if !map_crc != header.map_checksum {
            return Err(Error::Corrupt);
        }

        let mut receipt_bytes = [0u8; RECEIPT_BLOCK_BYTES];
        let mut receipts_crc = !0u32;
        let base = receipts_sector(header.generation);
        for sector in 0..RECEIPTS_SECTORS {
            disk.read(base + sector, &mut block)?;
            crc_update(&mut receipts_crc, &block);
            let at = sector as usize * 512;
            receipt_bytes[at..at + 512].copy_from_slice(&block);
        }
        if !receipts_crc != header.receipts_checksum {
            return Err(Error::Corrupt);
        }
        self.records = receipt_slots(&receipt_bytes)?;
        Ok(())
    }
}

fn select_header(copies: &[[u8; 512]; 2]) -> Result<(Header7, bool), Error> {
    let candidates = [candidate(&copies[0], 0), candidate(&copies[1], 1)];
    match candidates {
        [None, None] => Err(Error::Corrupt),
        [Some(header), None] => Ok((header, !clean_genesis(&header, &copies[1]))),
        [None, Some(header)] => Ok((header, true)),
        [Some(left), Some(right)] => {
            if left.lineage != right.lineage || left.generation == right.generation {
                return Err(Error::Corrupt);
            }
            let (older, newer) = if left.sequence < right.sequence {
                (left, right)
            } else if right.sequence < left.sequence {
                (right, left)
            } else {
                return Err(Error::Corrupt);
            };
            if newer.sequence.checked_sub(older.sequence) != Some(1)
                || newer.epoch < older.epoch
                || newer.next < older.next
            {
                return Err(Error::Corrupt);
            }
            Ok((newer, false))
        }
    }
}

fn candidate(bytes: &[u8; 512], physical_slot: u8) -> Option<Header7> {
    let header = Header7::decode(bytes).ok()?;
    (header.generation == physical_slot).then_some(header)
}

fn clean_genesis(header: &Header7, other_copy: &[u8; 512]) -> bool {
    header.generation == 0
        && header.sequence == Header7::INITIAL_SEQUENCE
        && header.epoch == Header7::INITIAL_EPOCH
        && header.next == Header7::NEXT_MIN
        && other_copy.iter().all(|byte| *byte == 0)
}
