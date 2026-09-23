// SPDX-License-Identifier: Apache-2.0
//! Namespace queries and copy-on-write metadata mutations for v7.

use crate::format7::{NAME_BYTES, NODES, Node7};
use crate::namespace::valid_name;
use crate::{Disk, Error, Kind};

use super::Volume7;
use super::payload::release_run;
use super::replacement::retained_owns_run;

impl Volume7 {
    /// Find one live node by persistent identity.
    pub fn stat(&self, id: u32) -> Result<Node7, Error> {
        self.ready()?;
        self.nodes
            .iter()
            .find(|node| node.kind != Kind::Empty && node.id == id)
            .copied()
            .ok_or(Error::NotFound)
    }

    /// Find a named child of a directory. Parent zero lists the four roots.
    pub fn lookup(&self, parent: u32, name: &[u8]) -> Result<Node7, Error> {
        self.ready()?;
        valid_name(name)?;
        if parent != 0 && self.stat(parent)?.kind != Kind::Directory {
            return Err(Error::NotDirectory);
        }
        self.nodes
            .iter()
            .find(|node| node.kind != Kind::Empty && node.parent == parent && node.name() == name)
            .copied()
            .ok_or(Error::NotFound)
    }

    /// Return the next live child and a cursor suitable for the following call.
    pub fn list(&self, parent: u32, cursor: usize) -> Result<Option<(usize, Node7)>, Error> {
        self.ready()?;
        if cursor > NODES {
            return Err(Error::Invalid);
        }
        if parent != 0 && self.stat(parent)?.kind != Kind::Directory {
            return Err(Error::NotDirectory);
        }
        Ok(self
            .nodes
            .iter()
            .enumerate()
            .skip(cursor)
            .find(|(_, node)| node.kind != Kind::Empty && node.parent == parent)
            .map(|(index, node)| (index + 1, *node)))
    }

    /// Create a file or directory with a new, never-reused identity.
    pub fn create(
        &mut self,
        disk: &mut impl Disk,
        parent: u32,
        name: &[u8],
        kind: Kind,
    ) -> Result<Node7, Error> {
        self.ready()?;
        valid_name(name)?;
        if kind == Kind::Empty {
            return Err(Error::Invalid);
        }

        let directory = self.stat(parent)?;
        if directory.space == 1 {
            return Err(Error::ReadOnly);
        }
        if directory.kind != Kind::Directory {
            return Err(Error::NotDirectory);
        }
        match self.lookup(parent, name) {
            Ok(_) => return Err(Error::Exists),
            Err(Error::NotFound) => (),
            Err(error) => return Err(error),
        }

        let slot = self
            .nodes
            .iter()
            .position(|node| node.kind == Kind::Empty)
            .ok_or(Error::Full)?;
        let id = self.header.next_id()?;
        let next_id = id.checked_add(1).ok_or(Error::Exhausted)?;
        let version = self
            .header
            .sequence
            .checked_add(1)
            .ok_or(Error::Exhausted)?;
        let mut name_field = [0; NAME_BYTES];
        name_field[..name.len()].copy_from_slice(name);
        let node = Node7 {
            id,
            parent,
            version,
            length: 0,
            kind,
            space: directory.space,
            extents_used: 0,
            extents: [crate::Extent::new(0, 0); crate::format7::MAX_EXTENTS],
            name_length: name.len() as u8,
            name: name_field,
            payload_crc32: 0,
        };

        self.fenced = true;
        self.nodes[slot] = node;
        self.header.next = next_id;
        self.publish_namespace(disk)?;
        Ok(node)
    }

    /// Remove a file or an empty directory while preserving retained snapshots.
    pub fn remove(&mut self, disk: &mut impl Disk, id: u32) -> Result<(), Error> {
        self.ready()?;
        let index = self
            .nodes
            .iter()
            .position(|node| node.kind != Kind::Empty && node.id == id)
            .ok_or(Error::NotFound)?;
        let node = self.nodes[index];
        if node.space == 1 || node.parent == 0 {
            return Err(Error::ReadOnly);
        }
        if self
            .nodes
            .iter()
            .any(|child| child.kind != Kind::Empty && child.parent == id)
        {
            return Err(Error::NotEmpty);
        }
        self.header
            .sequence
            .checked_add(1)
            .ok_or(Error::Exhausted)?;

        self.fenced = true;
        for run in node.runs() {
            if !retained_owns_run(&self.records, *run) && release_run(&mut self.map, *run).is_err()
            {
                self.clear();
                self.fenced = true;
                return Err(Error::Corrupt);
            }
        }
        self.nodes[index] = Node7::EMPTY;
        self.publish_namespace(disk)
    }

    fn publish_namespace(&mut self, disk: &mut impl Disk) -> Result<(), Error> {
        match self.publish_candidate(disk) {
            Ok(header) => {
                self.header = header;
                self.fenced = false;
                Ok(())
            }
            Err(error) => {
                self.clear();
                self.fenced = true;
                Err(error)
            }
        }
    }
}
