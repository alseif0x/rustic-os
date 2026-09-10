// SPDX-License-Identifier: Apache-2.0
//! Version-pinned range reads; validate stored bytes before publishing output.
use crate::{Disk, Error, Kind, Node, Volume, checksum::crc, storage};

impl Volume {
    pub fn read(
        &self,
        disk: &mut impl Disk,
        id: u32,
        offset: usize,
        output: &mut [u8],
    ) -> Result<usize, Error> {
        self.read_versioned(disk, id, None, offset, output)
            .map(|(_, count)| count)
    }

    /// The immutable volume borrow pins metadata through the complete read.
    /// On error, output remains untouched; a successful count may be zero at EOF.
    pub fn read_versioned(
        &self,
        disk: &mut impl Disk,
        id: u32,
        expected_version: Option<u64>,
        offset: usize,
        output: &mut [u8],
    ) -> Result<(Node, usize), Error> {
        let node = self.stat(id)?;
        if expected_version.is_some_and(|version| version != node.version) {
            return Err(Error::Version);
        }
        if node.kind != Kind::File {
            return Err(Error::IsDirectory);
        }
        if offset > usize::from(node.length) {
            return Err(Error::Size);
        }
        if node.length == 0 {
            return Ok((node, 0));
        }
        let bytes = storage::read_data(disk, self.metadata.index(id)?, node.bank)?;
        if crc(&bytes[..usize::from(node.length)]) != node.checksum {
            return Err(Error::Corrupt);
        }
        let count = output.len().min(usize::from(node.length) - offset);
        output[..count].copy_from_slice(&bytes[offset..offset + count]);
        Ok((node, count))
    }
}
