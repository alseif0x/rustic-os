// SPDX-License-Identifier: Apache-2.0
use crate::{
    Disk, Error, Kind, MAX_FILE, Node, Publication, Volume, checksum::crc, format::Metadata,
    namespace::valid_name, storage,
};
impl Volume {
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
    pub fn replace(
        &mut self,
        disk: &mut impl Disk,
        id: u32,
        version: u64,
        bytes: &[u8],
    ) -> Result<Node, Error> {
        self.prepare_replace(disk, id, version, bytes)?.run()
    }
    /// Prepare a volatile write without issuing I/O. The returned owner permits
    /// cancellation between commands, before the publication header is submitted.
    pub fn prepare_replace<'a, D: Disk>(
        &'a mut self,
        disk: &'a mut D,
        id: u32,
        version: u64,
        bytes: &[u8],
    ) -> Result<Publication<'a, D, Node>, Error> {
        self.prepare_recorded(disk, id, version, bytes, self.metadata.clone(), |node| node)
    }
    pub(crate) fn replace_recorded(
        &mut self,
        disk: &mut impl Disk,
        id: u32,
        version: u64,
        bytes: &[u8],
        next: Metadata,
    ) -> Result<Node, Error> {
        self.prepare_recorded(disk, id, version, bytes, next, |node| node)?
            .run()
    }
    pub(crate) fn prepare_recorded<'a, D: Disk, T: Copy>(
        &'a mut self,
        disk: &'a mut D,
        id: u32,
        version: u64,
        bytes: &[u8],
        mut next: Metadata,
        output: impl FnOnce(Node) -> T,
    ) -> Result<Publication<'a, D, T>, Error> {
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
        next.sequence = result.version;
        Ok(Publication::new(
            self,
            disk,
            next,
            storage::data(slot, result.bank),
            bytes,
            output(result),
        ))
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
