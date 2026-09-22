// SPDX-License-Identifier: Apache-2.0
//! Canonical v7 format initialization and first-header publication.

use crate::checksum::crc_update;
use crate::format7::{
    Header7, MAP_SECTORS, NODE_BYTES, NODES_SECTORS, RECEIPTS_SECTORS, header_sector, map_sector,
    nodes_sector, receipts_sector,
};
use crate::{Disk, Error, Kind};

use super::Volume7;

const ROOTS: [&[u8]; 4] = [b"system", b"data", b"config", b"workspaces"];

impl Volume7 {
    /// Destructively format a fresh or disposable v7 volume in place.
    ///
    /// Both header slots are zeroed and flushed before generation 0 is written.
    /// Canonical roots, map and receipts are then flushed before header 0 is
    /// written and flushed. This method does not publish later generations or
    /// provide mutation, migration, service, or crash-atomicity guarantees for
    /// media that can tear a sector write. It overwrites existing metadata and
    /// is not an upgrade path; never use it to preserve an existing volume.
    pub fn provision_into(&mut self, disk: &mut impl Disk, lineage: [u8; 16]) -> Result<(), Error> {
        let initial = Header7::initial(lineage);
        initial.encode()?;

        self.fenced = true;
        self.clear();
        self.install_roots();
        let result = self.provision_disk(disk, initial);
        match result {
            Ok(header) => {
                self.header = header;
                self.fenced = false;
                Ok(())
            }
            Err(error) => {
                self.clear();
                Err(error)
            }
        }
    }

    fn install_roots(&mut self) {
        for (index, name) in ROOTS.iter().enumerate() {
            let mut field = [0; crate::format7::NAME_BYTES];
            field[..name.len()].copy_from_slice(name);
            self.nodes[index] = crate::format7::Node7 {
                id: index as u32 + 1,
                parent: 0,
                version: 1,
                length: 0,
                kind: Kind::Directory,
                space: index as u8 + 1,
                extents_used: 0,
                extents: [crate::Extent::new(0, 0); crate::format7::MAX_EXTENTS],
                name_length: name.len() as u8,
                name: field,
                payload_crc32: 0,
            };
        }
    }

    fn provision_disk(&self, disk: &mut impl Disk, initial: Header7) -> Result<Header7, Error> {
        let invalid = [0u8; 512];
        for generation in 0..2 {
            disk.write(header_sector(generation), &invalid)?;
        }
        disk.flush()?;

        let (nodes_checksum, map_checksum, receipts_checksum) = self.write_generation_zero(disk)?;
        disk.flush()?;

        let header = Header7 {
            nodes_checksum,
            map_checksum,
            receipts_checksum,
            ..initial
        };
        let bytes = header.encode()?;
        disk.write(header_sector(0), &bytes)?;
        disk.flush()?;
        Ok(header)
    }

    fn write_generation_zero(&self, disk: &mut impl Disk) -> Result<(u32, u32, u32), Error> {
        let nodes_base = nodes_sector(0);
        let mut nodes_crc = !0u32;
        let mut block = [0u8; 512];
        for sector in 0..NODES_SECTORS {
            block.fill(0);
            for offset in 0..512 / NODE_BYTES {
                let index = sector as usize * (512 / NODE_BYTES) + offset;
                let encoded = self.nodes[index].encode()?;
                block[offset * NODE_BYTES..(offset + 1) * NODE_BYTES].copy_from_slice(&encoded);
            }
            crc_update(&mut nodes_crc, &block);
            disk.write(nodes_base + sector, &block)?;
        }

        let mut map_crc = !0u32;
        let map_base = map_sector(0);
        for sector in 0..MAP_SECTORS {
            block.fill(0);
            for offset in 0..512 / 8 {
                let index = sector as usize * (512 / 8) + offset;
                block[offset * 8..(offset + 1) * 8].copy_from_slice(&self.map[index].to_le_bytes());
            }
            crc_update(&mut map_crc, &block);
            disk.write(map_base + sector, &block)?;
        }

        let mut receipts_crc = !0u32;
        let receipts_base = receipts_sector(0);
        for sector in 0..RECEIPTS_SECTORS {
            block.fill(0);
            crc_update(&mut receipts_crc, &block);
            disk.write(receipts_base + sector, &block)?;
        }

        Ok((!nodes_crc, !map_crc, !receipts_crc))
    }
}
