// SPDX-License-Identifier: Apache-2.0
use super::{Key, OperationId};
use crate::files::{
    reference::{Epoch, Resource, Version, Workspace},
    *,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retry {
    pub epoch: Epoch,
    pub key: Key,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Replacement {
    pub workspace: Workspace,
    pub resource: Resource,
    pub expected_version: Version,
    pub retry: Retry,
}
impl Replacement {
    pub fn packet(self, size: usize, context: u32) -> Result<Packet, Error> {
        if self.workspace != self.resource.workspace() {
            return Err(Error::Denied);
        }
        if size > 1024 {
            return Err(Error::Size);
        }
        let mut p = Packet::new(REPLACE_OPEN);
        p.id = self.resource.object();
        p.arg = size as u32;
        p.version = self.expected_version.value();
        p.context = context;
        p.count = 36;
        p.data[..16].copy_from_slice(&self.workspace.lineage());
        p.data[16..20].copy_from_slice(&self.workspace.root().to_le_bytes());
        p.data[20..28].copy_from_slice(&self.retry.epoch.value().to_le_bytes());
        p.data[28..36].copy_from_slice(&self.retry.key.value().to_le_bytes());
        Ok(p)
    }
    pub fn decode(p: &Packet) -> Result<Self, Error> {
        if !matches!(p.op, REPLACE_OPEN | admission::OPEN)
            || p.status != 0
            || p.count != 36
            || p.arg > 1024
            || p.data[36..] != [0; 4]
        {
            return Err(Error::Protocol);
        }
        let workspace = Workspace::new(
            p.data[..16].try_into().unwrap(),
            u32::from_le_bytes(p.data[16..20].try_into().unwrap()),
        )?;
        Ok(Self {
            workspace,
            resource: Resource::new(workspace, p.id)?,
            expected_version: Version::new(p.version)?,
            retry: Retry {
                epoch: Epoch::new(u64::from_le_bytes(p.data[20..28].try_into().unwrap()))?,
                key: Key::new(u64::from_le_bytes(p.data[28..36].try_into().unwrap()))?,
            },
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Lookup {
    Retry { workspace: Workspace, retry: Retry },
    Id(OperationId),
}
impl Lookup {
    pub fn packet(self, context: u32) -> Packet {
        let mut p = Packet::new(match self {
            Self::Retry { .. } => OPERATION_RETRY,
            Self::Id(_) => OPERATION_ID,
        });
        p.context = context;
        match self {
            Self::Retry { workspace, retry } => {
                p.id = workspace.root();
                p.version = retry.epoch.value();
                p.count = 24;
                p.data[..16].copy_from_slice(&workspace.lineage());
                p.data[16..24].copy_from_slice(&retry.key.value().to_le_bytes());
            }
            Self::Id(id) => {
                p.version = id.sequence();
                p.count = 16;
                p.data[..16].copy_from_slice(&id.lineage());
            }
        }
        p
    }
    pub fn decode(p: &Packet) -> Result<Self, Error> {
        match p.op {
            OPERATION_RETRY if p.arg == 0 && p.count == 24 => Ok(Self::Retry {
                workspace: Workspace::new(p.data[..16].try_into().unwrap(), p.id)?,
                retry: Retry {
                    epoch: Epoch::new(p.version)?,
                    key: Key::new(u64::from_le_bytes(p.data[16..24].try_into().unwrap()))?,
                },
            }),
            OPERATION_ID | OPERATION_PART if p.id == 0 && p.count == 16 => Ok(Self::Id(
                OperationId::new(p.data[..16].try_into().unwrap(), p.version)?,
            )),
            _ => Err(Error::Protocol),
        }
    }
}
