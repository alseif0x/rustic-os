// SPDX-License-Identifier: Apache-2.0
//! Stable object identities. These values identify objects; they confer no authority.
pub(crate) mod text;
use super::{Error, Packet, REFERENCES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Workspace {
    lineage: [u8; 16],
    root: u32,
}
impl Workspace {
    pub fn new(lineage: [u8; 16], root: u32) -> Result<Self, Error> {
        if lineage == [0; 16] || root == 0 {
            return Err(Error::Invalid);
        }
        Ok(Self { lineage, root })
    }
    pub const fn lineage(self) -> [u8; 16] {
        self.lineage
    }
    pub const fn root(self) -> u32 {
        self.root
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resource {
    workspace: Workspace,
    object: u32,
}
impl Resource {
    pub fn new(workspace: Workspace, object: u32) -> Result<Self, Error> {
        if object == 0 {
            return Err(Error::Invalid);
        }
        Ok(Self { workspace, object })
    }
    pub const fn workspace(self) -> Workspace {
        self.workspace
    }
    pub const fn object(self) -> u32 {
        self.object
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct References {
    pub workspace: Workspace,
    pub resource: Resource,
}
impl References {
    pub fn new(lineage: [u8; 16], workspace: u32, object: u32) -> Result<Self, Error> {
        let workspace = Workspace::new(lineage, workspace)?;
        Ok(Self {
            workspace,
            resource: Resource::new(workspace, object)?,
        })
    }
    pub fn request(workspace: u32, object: u32, context: u32) -> Result<Packet, Error> {
        if workspace == 0 || object == 0 {
            return Err(Error::Invalid);
        }
        let mut p = Packet::new(REFERENCES);
        p.id = object;
        p.arg = workspace;
        p.context = context;
        Ok(p)
    }
    /// Reply to the native identity bootstrap operation, not live discovery.
    pub fn packet(self, context: u32) -> Result<Packet, Error> {
        if self.workspace != self.resource.workspace {
            return Err(Error::Invalid);
        }
        let mut p = Self::request(self.workspace.root, self.resource.object, context)?;
        p.count = 16;
        p.data[..16].copy_from_slice(&self.workspace.lineage);
        Ok(p)
    }
    pub fn decode(p: &Packet) -> Result<Self, Error> {
        if p.op != REFERENCES
            || p.status != 0
            || p.version != 0
            || p.count != 16
            || p.data[16..].iter().any(|b| *b != 0)
        {
            return Err(Error::Protocol);
        }
        Self::new(p.data[..16].try_into().unwrap(), p.arg, p.id).map_err(|_| Error::Protocol)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Version(u64);
impl Version {
    pub fn new(value: u64) -> Result<Self, Error> {
        if value == 0 {
            Err(Error::Invalid)
        } else {
            Ok(Self(value))
        }
    }
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Epoch(u64);
impl Epoch {
    pub fn new(value: u64) -> Result<Self, Error> {
        if value == 0 {
            Err(Error::Invalid)
        } else {
            Ok(Self(value))
        }
    }
    pub const fn value(self) -> u64 {
        self.0
    }
}
