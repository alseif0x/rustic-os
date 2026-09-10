// SPDX-License-Identifier: Apache-2.0
use super::{MAX_INTEGER, MAX_RANGE, VERSION};
use crate::files::{
    DATA, Error, Packet, READ_CHUNK, READ_OPEN,
    reference::{Resource, Version, Workspace},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub workspace: Workspace,
    pub resource: Resource,
    pub expected_version: Option<Version>,
    pub offset: u64,
    pub length: u16,
}
impl Request {
    pub fn validate(&self) -> Result<(), Error> {
        if self.workspace != self.resource.workspace()
            || self.length == 0
            || usize::from(self.length) > MAX_RANGE
            || self
                .offset
                .checked_add(u64::from(self.length))
                .is_none_or(|end| end > MAX_INTEGER)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn packet(self, op: u8, context: u32) -> Result<Packet, Error> {
        self.validate()?;
        if !matches!(op, READ_OPEN | READ_CHUNK)
            || op == READ_CHUNK
                && (self.expected_version.is_none() || usize::from(self.length) > DATA)
        {
            return Err(Error::Invalid);
        }
        let mut p = Packet::new(op);
        p.id = self.resource.object();
        p.arg = u32::from(self.length);
        p.context = context;
        p.version = self.expected_version.map_or(0, Version::value);
        p.count = 30;
        p.data[..16].copy_from_slice(&self.workspace.lineage());
        p.data[16..20].copy_from_slice(&self.workspace.root().to_le_bytes());
        p.data[20..28].copy_from_slice(&self.offset.to_le_bytes());
        p.data[28..30].copy_from_slice(&VERSION.to_le_bytes());
        Ok(p)
    }
    pub fn decode(p: &Packet) -> Result<Self, Error> {
        if !matches!(p.op, READ_OPEN | READ_CHUNK)
            || p.status != 0
            || p.count != 30
            || p.data[30..].iter().any(|b| *b != 0)
        {
            return Err(Error::Invalid);
        }
        if u16::from_le_bytes(p.data[28..30].try_into().unwrap()) != VERSION {
            return Err(Error::UnsupportedVersion);
        }
        let workspace = Workspace::new(
            p.data[..16].try_into().unwrap(),
            u32::from_le_bytes(p.data[16..20].try_into().unwrap()),
        )?;
        let result = Self {
            workspace,
            resource: Resource::new(workspace, p.id)?,
            expected_version: if p.version == 0 {
                None
            } else {
                Some(Version::new(p.version)?)
            },
            offset: u64::from_le_bytes(p.data[20..28].try_into().unwrap()),
            length: u16::try_from(p.arg).map_err(|_| Error::Invalid)?,
        };
        result.packet(p.op, p.context)?;
        Ok(result)
    }
}
