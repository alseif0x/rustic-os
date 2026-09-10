// SPDX-License-Identifier: Apache-2.0
use crate::{
    Disk, Error, Kind, MAX_FILE, Node, OBJECTS, SECTORS, checksum::crc, format::Metadata,
    namespace::valid_name, storage,
};
pub struct Volume {
    metadata: Metadata,
    bank: u8,
    poisoned: bool,
}
impl Volume {
    pub fn mount(disk: &mut impl Disk) -> Result<Self, Error> {
        let a = Metadata::read(disk, 0);
        let b = Metadata::read(disk, 1);
        // A read error cannot safely be treated as an invalid/absent generation.
        for result in [&a, &b] {
            if let Err(error) = result
                && !matches!(error, Error::Empty | Error::Corrupt)
            {
                return Err(*error);
            }
        }
        let (metadata, bank) = match (a, b) {
            (Ok(a), Ok(b)) if a.sequence == b.sequence => {
                if a.bytes() != b.bytes() || a.next != b.next {
                    return Err(Error::Corrupt);
                }
                (a, 0)
            }
            (Ok(a), Ok(b)) => {
                if a.sequence > b.sequence {
                    (a, 0)
                } else {
                    (b, 1)
                }
            }
            (Ok(a), Err(_)) => (a, 0),
            (Err(_), Ok(b)) => (b, 1),
            (Err(Error::Empty), Err(Error::Empty)) => return Err(Error::Empty),
            _ => return Err(Error::Corrupt),
        };
        Ok(Self {
            metadata,
            bank,
            poisoned: false,
        })
    }
    /// Only for an explicitly selected fresh volume; rejects any nonzero reserved sector.
    pub fn initialize(disk: &mut impl Disk) -> Result<Self, Error> {
        for sector in 0..SECTORS {
            let mut bytes = [0; 512];
            disk.read(sector, &mut bytes)?;
            if bytes != [0; 512] {
                return Err(Error::Corrupt);
            }
        }
        let metadata = Metadata::initial();
        metadata.write(disk, 0).map_err(|_| Error::Uncertain)?;
        Ok(Self {
            metadata,
            bank: 0,
            poisoned: false,
        })
    }
    fn ready(&self) -> Result<(), Error> {
        if self.poisoned {
            Err(Error::Uncertain)
        } else {
            Ok(())
        }
    }
    pub fn sequence(&self) -> u64 {
        self.metadata.sequence
    }
    pub fn stat(&self, id: u32) -> Result<Node, Error> {
        self.ready()?;
        Ok(self.metadata.nodes[self.metadata.index(id)?])
    }
    pub fn lookup(&self, parent: u32, name: &[u8]) -> Result<Node, Error> {
        self.ready()?;
        valid_name(name)?;
        if parent != 0 && self.stat(parent)?.kind != Kind::Directory {
            return Err(Error::NotDirectory);
        }
        self.metadata
            .nodes
            .iter()
            .find(|n| n.kind != Kind::Empty && n.parent == parent && n.name() == name)
            .copied()
            .ok_or(Error::NotFound)
    }
    pub fn list(&self, parent: u32, cursor: usize) -> Result<Option<(usize, Node)>, Error> {
        self.ready()?;
        if cursor > OBJECTS {
            return Err(Error::Invalid);
        }
        if parent != 0 && self.stat(parent)?.kind != Kind::Directory {
            return Err(Error::NotDirectory);
        }
        Ok(self
            .metadata
            .nodes
            .iter()
            .enumerate()
            .skip(cursor)
            .find(|(_, n)| n.kind != Kind::Empty && n.parent == parent)
            .map(|(i, n)| (i + 1, *n)))
    }
    pub fn within(&self, id: u32, scope: u32) -> bool {
        if scope == 0 {
            return self.stat(id).is_ok();
        }
        let mut current = id;
        for _ in 0..OBJECTS {
            let Ok(n) = self.stat(current) else {
                return false;
            };
            if current == scope {
                return true;
            }
            if n.parent == 0 {
                return false;
            }
            current = n.parent;
        }
        false
    }
    fn writable(&self, id: u32) -> Result<Node, Error> {
        let n = self.stat(id)?;
        if n.space == 1 {
            Err(Error::ReadOnly)
        } else {
            Ok(n)
        }
    }
    fn commit(&mut self, disk: &mut impl Disk, mut next: Metadata) -> Result<(), Error> {
        next.sequence = self
            .metadata
            .sequence
            .checked_add(1)
            .ok_or(Error::Exhausted)?;
        if next.write(disk, 1 - self.bank).is_err() {
            self.poisoned = true;
            return Err(Error::Uncertain);
        }
        self.metadata = next;
        self.bank = 1 - self.bank;
        Ok(())
    }
    pub fn create(
        &mut self,
        disk: &mut impl Disk,
        parent: u32,
        name: &[u8],
        kind: Kind,
    ) -> Result<Node, Error> {
        valid_name(name)?;
        if kind == Kind::Empty {
            return Err(Error::Invalid);
        }
        let directory = self.writable(parent)?;
        if directory.kind != Kind::Directory {
            return Err(Error::NotDirectory);
        }
        match self.lookup(parent, name) {
            Ok(_) => return Err(Error::Exists),
            Err(Error::NotFound) => {}
            Err(e) => return Err(e),
        }
        let slot = self
            .metadata
            .nodes
            .iter()
            .position(|n| n.kind == Kind::Empty)
            .ok_or(Error::Full)?;
        let mut next = self.metadata.clone();
        let id = next.next;
        next.next = next.next.checked_add(1).ok_or(Error::Exhausted)?;
        let node = Node::new(
            id,
            parent,
            directory.space,
            kind,
            name,
            self.metadata
                .sequence
                .checked_add(1)
                .ok_or(Error::Exhausted)?,
        )?;
        next.nodes[slot] = node;
        self.commit(disk, next)?;
        Ok(node)
    }
    pub fn read(
        &self,
        disk: &mut impl Disk,
        id: u32,
        offset: usize,
        output: &mut [u8],
    ) -> Result<usize, Error> {
        let node = self.stat(id)?;
        if node.kind != Kind::File {
            return Err(Error::IsDirectory);
        }
        if offset > usize::from(node.length) {
            return Err(Error::Size);
        }
        if node.length == 0 {
            return Ok(0);
        }
        let bytes = storage::read_data(disk, self.metadata.index(id)?, node.bank)?;
        if crc(&bytes[..usize::from(node.length)]) != node.checksum {
            return Err(Error::Corrupt);
        }
        let length = output.len().min(usize::from(node.length) - offset);
        output[..length].copy_from_slice(&bytes[offset..offset + length]);
        Ok(length)
    }
    pub fn replace(
        &mut self,
        disk: &mut impl Disk,
        id: u32,
        version: u64,
        bytes: &[u8],
    ) -> Result<Node, Error> {
        let node = self.writable(id)?;
        if node.kind != Kind::File {
            return Err(Error::IsDirectory);
        }
        if node.version != version {
            return Err(Error::Version);
        }
        if bytes.len() > MAX_FILE {
            return Err(Error::Size);
        }
        let slot = self.metadata.index(id)?;
        let mut next = self.metadata.clone();
        let changed = &mut next.nodes[slot];
        changed.bank = 1 - node.bank;
        changed.length = bytes.len() as u16;
        changed.checksum = crc(bytes);
        changed.version = self
            .metadata
            .sequence
            .checked_add(1)
            .ok_or(Error::Exhausted)?;
        let result = *changed;
        for i in 0..2 {
            let mut sector = [0; 512];
            let start = i * 512;
            let count = bytes.len().saturating_sub(start).min(512);
            if count > 0 {
                sector[..count].copy_from_slice(&bytes[start..start + count]);
            }
            if disk
                .write(storage::data(slot, result.bank) + i as u64, &sector)
                .is_err()
            {
                self.poisoned = true;
                return Err(Error::Uncertain);
            }
        }
        if disk.flush().is_err() {
            self.poisoned = true;
            return Err(Error::Uncertain);
        }
        self.commit(disk, next)?;
        Ok(result)
    }
    pub fn remove(&mut self, disk: &mut impl Disk, id: u32) -> Result<(), Error> {
        let node = self.writable(id)?;
        if node.parent == 0 {
            return Err(Error::ReadOnly);
        }
        if self
            .metadata
            .nodes
            .iter()
            .any(|n| n.kind != Kind::Empty && n.parent == id)
        {
            return Err(Error::NotEmpty);
        }
        let mut next = self.metadata.clone();
        next.nodes[self.metadata.index(id)?] = Node::EMPTY;
        self.commit(disk, next)
    }
}
