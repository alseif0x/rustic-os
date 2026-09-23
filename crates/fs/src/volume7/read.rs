// SPDX-License-Identifier: Apache-2.0
//! Bounded reads from the live payload version verified by a mounted v7 owner.

use crate::format7::{self, SECTOR_BYTES};
use crate::{Disk, Error, Kind};

use super::Volume7;

impl Volume7 {
    /// Copy at most `out.len()` bytes from a file's current mounted version.
    ///
    /// An expected version mismatch, directory, past-EOF offset, or malformed
    /// node is refused before payload I/O. Mount verifies each live payload CRC;
    /// this bounded read trusts that verification and does not rescan the whole
    /// file. Changes made to the medium outside this owner after mount are not
    /// detected by the range read. An I/O error may leave a prefix in `out`.
    pub fn read_range(
        &self,
        disk: &mut impl Disk,
        id: u32,
        expected_version: Option<u64>,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, Error> {
        let node = self.stat(id)?;
        if expected_version.is_some_and(|version| version != node.version) {
            return Err(Error::Version);
        }
        if node.kind != Kind::File {
            return Err(Error::IsDirectory);
        }
        node.validate().map_err(|_| Error::Corrupt)?;

        let length = u64::from(node.length);
        if offset > length {
            return Err(Error::Size);
        }
        let count = out.len().min((length - offset) as usize);
        let end = offset + count as u64;
        let mut file_position = 0u64;
        let mut written = 0usize;
        let mut block = [0u8; 512];

        for run in node.runs() {
            let run_bytes = run
                .sectors
                .checked_mul(SECTOR_BYTES)
                .ok_or(Error::Corrupt)?;
            let run_end = file_position.checked_add(run_bytes).ok_or(Error::Corrupt)?;
            if run_end > offset && file_position < end {
                let from = offset.max(file_position);
                let to = end.min(run_end);
                let mut sector = (from - file_position) / SECTOR_BYTES;
                let mut within = (from - file_position) % SECTOR_BYTES;
                let mut remaining = (to - from) as usize;

                while remaining > 0 {
                    let disk_sector = format7::PAYLOAD_SECTOR
                        .checked_add(run.start)
                        .and_then(|base| base.checked_add(sector))
                        .ok_or(Error::Corrupt)?;
                    disk.read(disk_sector, &mut block)?;
                    let take = remaining.min(SECTOR_BYTES as usize - within as usize);
                    out[written..written + take]
                        .copy_from_slice(&block[within as usize..within as usize + take]);
                    written += take;
                    remaining -= take;
                    sector += 1;
                    within = 0;
                }
            }
            file_position = run_end;
        }

        if written != count {
            return Err(Error::Corrupt);
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extent::{DATA_SECTORS, Extent};
    use crate::format7::Node7;

    #[derive(Default)]
    struct IoCounts {
        reads: usize,
        writes: usize,
    }

    impl Disk for IoCounts {
        fn read(&mut self, _: u64, _: &mut [u8; 512]) -> Result<(), Error> {
            self.reads += 1;
            Ok(())
        }

        fn write(&mut self, _: u64, _: &[u8; 512]) -> Result<(), Error> {
            self.writes += 1;
            Ok(())
        }

        fn flush(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }

    fn volume_with_file(length: u32, extents: [Extent; format7::MAX_EXTENTS], used: u8) -> Volume7 {
        let mut volume = Volume7::EMPTY;
        let mut name = [0; format7::NAME_BYTES];
        name[0] = b'x';
        volume.fenced = false;
        volume.nodes[4] = Node7 {
            id: 5,
            parent: 4,
            version: 1,
            length,
            kind: Kind::File,
            space: 2,
            extents_used: used,
            extents,
            name_length: 1,
            name,
            payload_crc32: 0,
        };
        volume
    }

    #[test]
    fn malformed_extent_geometry_is_refused_before_payload_io() {
        let mut out = [0u8; 1];
        let mut out_of_bounds = [Extent::new(0, 0); format7::MAX_EXTENTS];
        out_of_bounds[0] = Extent::new(DATA_SECTORS, 1);
        let mut overlapping = [Extent::new(0, 0); format7::MAX_EXTENTS];
        overlapping[0] = Extent::new(10, 1);
        overlapping[1] = Extent::new(10, 1);
        let mut undersized = [Extent::new(0, 0); format7::MAX_EXTENTS];
        undersized[0] = Extent::new(20, 1);

        for (length, extents, used) in [
            (1, out_of_bounds, 1),
            (513, overlapping, 2),
            (513, undersized, 1),
        ] {
            let volume = volume_with_file(length, extents, used);
            let mut disk = IoCounts::default();
            assert_eq!(
                volume.read_range(&mut disk, 5, None, 0, &mut out),
                Err(Error::Corrupt)
            );
            assert_eq!(disk.reads, 0);
            assert_eq!(disk.writes, 0);
        }
    }
}
