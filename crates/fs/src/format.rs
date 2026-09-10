// SPDX-License-Identifier: Apache-2.0
use crate::{Disk, Error, Kind, Node, OBJECTS, checksum::crc, storage::header};
#[derive(Clone)]
pub(super) struct Metadata {
    pub(super) sequence: u64,
    pub(super) next: u32,
    pub(super) nodes: [Node; OBJECTS],
    pub(crate) recovery: Option<crate::recovery::Recovery>,
}
impl Metadata {
    pub(super) fn initial() -> Self {
        let mut result = Self {
            sequence: 1,
            next: 5,
            nodes: [Node::EMPTY; OBJECTS],
            recovery: None,
        };
        for (i, name) in [b"system".as_slice(), b"data", b"config", b"workspaces"]
            .iter()
            .enumerate()
        {
            result.nodes[i] =
                Node::new(i as u32 + 1, 0, i as u8 + 1, Kind::Directory, name, 1).unwrap();
        }
        result
    }
    pub(super) fn index(&self, id: u32) -> Result<usize, Error> {
        self.nodes
            .iter()
            .position(|n| n.id == id && n.kind != Kind::Empty)
            .ok_or(Error::NotFound)
    }
    pub(super) fn bytes(&self) -> [u8; 2048] {
        let mut bytes = [0; 2048];
        for (chunk, node) in bytes.as_chunks_mut::<64>().0.iter_mut().zip(self.nodes) {
            chunk.copy_from_slice(&node.encode());
        }
        bytes
    }
    pub(super) fn write(&self, disk: &mut impl Disk, bank: u8) -> Result<(), Error> {
        let bytes = self.bytes();
        let recovery_crc = if let Some(recovery) = &self.recovery {
            let records = recovery.encode();
            for (i, chunk) in records.as_chunks::<512>().0.iter().enumerate() {
                disk.write(160 + u64::from(bank) * 7 + i as u64, chunk)?;
            }
            crate::checksum::crc(&records)
        } else {
            0
        };
        for (i, chunk) in bytes.as_chunks::<512>().0.iter().enumerate() {
            disk.write(header(bank) + 1 + i as u64, chunk)?;
        }
        disk.flush()?;
        let mut b = [0; 512];
        b[..8].copy_from_slice(b"RUSTFS1\0");
        b[8..10]
            .copy_from_slice(&(if self.recovery.is_some() { 2u16 } else { 1u16 }).to_le_bytes());
        b[32..36].copy_from_slice(&recovery_crc.to_le_bytes());
        b[10..12].copy_from_slice(&512u16.to_le_bytes());
        b[12..20].copy_from_slice(&self.sequence.to_le_bytes());
        b[20..24].copy_from_slice(&self.next.to_le_bytes());
        b[24..28].copy_from_slice(&crc(&bytes).to_le_bytes());
        let hash = crc(&b);
        b[28..32].copy_from_slice(&hash.to_le_bytes());
        disk.write(header(bank), &b)?;
        disk.flush()
    }
    pub(super) fn read(disk: &mut impl Disk, bank: u8) -> Result<Self, Error> {
        let mut b = [0; 512];
        disk.read(header(bank), &mut b)?;
        if b == [0; 512] {
            return Err(Error::Empty);
        }
        if &b[..8] != b"RUSTFS1\0"
            || !matches!(b[8], 1 | 2)
            || b[9..12] != [0, 0, 2]
            || b[36..].iter().any(|b| *b != 0)
            || b[8] == 1 && b[32..36] != [0; 4]
        {
            return Err(Error::Corrupt);
        }
        let checksum = u32::from_le_bytes(b[28..32].try_into().unwrap());
        b[28..32].fill(0);
        if checksum != crc(&b) {
            return Err(Error::Corrupt);
        }
        let mut result = Self {
            sequence: u64::from_le_bytes(b[12..20].try_into().unwrap()),
            next: u32::from_le_bytes(b[20..24].try_into().unwrap()),
            nodes: [Node::EMPTY; OBJECTS],
            recovery: None,
        };
        let mut bytes = [0; 2048];
        for (i, chunk) in bytes.as_chunks_mut::<512>().0.iter_mut().enumerate() {
            disk.read(header(bank) + 1 + i as u64, chunk)?;
        }
        if crc(&bytes) != u32::from_le_bytes(b[24..28].try_into().unwrap()) {
            return Err(Error::Corrupt);
        }
        for (node, chunk) in result
            .nodes
            .iter_mut()
            .zip(bytes.as_chunks::<64>().0.iter())
        {
            *node = Node::decode(chunk)?;
        }
        if b[8] == 2 {
            let mut records = [0; crate::recovery::RECOVERY_SECTORS * 512];
            for (i, chunk) in records.as_chunks_mut::<512>().0.iter_mut().enumerate() {
                disk.read(160 + u64::from(bank) * 7 + i as u64, chunk)?;
            }
            if crc(&records) != u32::from_le_bytes(b[32..36].try_into().unwrap()) {
                return Err(Error::Corrupt);
            }
            result.recovery = Some(crate::recovery::Recovery::decode(
                &records,
                result.sequence,
                result.next,
            )?);
        }
        result.validate()?;
        Ok(result)
    }
    fn validate(&self) -> Result<(), Error> {
        if self.sequence == 0 || self.next < 5 {
            return Err(Error::Corrupt);
        }
        for (index, node) in self.nodes.iter().enumerate() {
            if node.kind == Kind::Empty {
                continue;
            }
            if node.id >= self.next || node.version > self.sequence {
                return Err(Error::Corrupt);
            }
            if node.parent == 0 {
                let names = [b"system".as_slice(), b"data", b"config", b"workspaces"];
                if node.id > 4
                    || node.kind != Kind::Directory
                    || node.space != node.id as u8
                    || node.name() != names[node.id as usize - 1]
                {
                    return Err(Error::Corrupt);
                }
            } else {
                let parent = &self.nodes[self.index(node.parent).map_err(|_| Error::Corrupt)?];
                if parent.kind != Kind::Directory || parent.space != node.space {
                    return Err(Error::Corrupt);
                }
                let mut ancestor = node.parent;
                for depth in 0..OBJECTS {
                    if ancestor == node.id || depth == OBJECTS - 1 {
                        return Err(Error::Corrupt);
                    }
                    let a = &self.nodes[self.index(ancestor).map_err(|_| Error::Corrupt)?];
                    if a.parent == 0 {
                        break;
                    }
                    ancestor = a.parent;
                }
            }
            if self.nodes[..index].iter().any(|n| {
                n.kind != Kind::Empty
                    && (n.id == node.id || (n.parent == node.parent && n.name() == node.name()))
            }) {
                return Err(Error::Corrupt);
            }
        }
        for id in 1..=4 {
            let n = &self.nodes[self.index(id).map_err(|_| Error::Corrupt)?];
            if n.parent != 0 {
                return Err(Error::Corrupt);
            }
        }
        Ok(())
    }
}
