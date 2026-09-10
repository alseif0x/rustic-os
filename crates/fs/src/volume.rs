// SPDX-License-Identifier: Apache-2.0
use crate::{Disk, Error, Kind, Node, OBJECTS, SECTORS, format::Metadata, namespace::valid_name};
pub struct Volume {
    pub(crate) metadata: Metadata,
    pub(crate) bank: u8,
    pub(crate) poisoned: bool,
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
                if a.bytes() != b.bytes()
                    || a.next != b.next
                    || a.recovery.as_ref().map(|r| r.encode())
                        != b.recovery.as_ref().map(|r| r.encode())
                {
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
        let mut volume = Self {
            metadata,
            bank,
            poisoned: false,
        };
        if let Some(lineage) = crate::provision::lineage(disk)? {
            volume.enable_recovery(disk, lineage)?;
        }
        Ok(volume)
    }
    /// Only for an explicitly selected fresh volume; rejects any nonzero reserved sector.
    pub fn initialize(disk: &mut impl Disk) -> Result<Self, Error> {
        let lineage = crate::provision::lineage(disk)?;
        for sector in 0..SECTORS {
            if sector == 1 && lineage.is_some() {
                continue;
            }
            let mut bytes = [0; 512];
            disk.read(sector, &mut bytes)?;
            if bytes != [0; 512] {
                return Err(Error::Corrupt);
            }
        }
        let mut metadata = Metadata::initial();
        metadata.recovery = lineage.map(crate::recovery::Recovery::new).transpose()?;
        metadata.write(disk, 0).map_err(|_| Error::Uncertain)?;
        Ok(Self {
            metadata,
            bank: 0,
            poisoned: false,
        })
    }
    pub(crate) fn ready(&self) -> Result<(), Error> {
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
    pub(crate) fn writable(&self, id: u32) -> Result<Node, Error> {
        let n = self.stat(id)?;
        if n.space == 1 {
            Err(Error::ReadOnly)
        } else {
            Ok(n)
        }
    }
    pub(crate) fn commit(&mut self, disk: &mut impl Disk, mut next: Metadata) -> Result<(), Error> {
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
}
