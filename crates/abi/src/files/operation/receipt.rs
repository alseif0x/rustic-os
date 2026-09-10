// SPDX-License-Identifier: Apache-2.0
//! Three stateless fragments of one canonical completed-operation receipt.
use super::{Instance, Key, OperationId, Retry};
use crate::files::{
    DATA, Error, Packet,
    reference::{Epoch, Resource, Version, Workspace},
};
pub const RECEIPT_BYTES: usize = 104;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operation {
    pub id: OperationId,
    pub service_instance: Instance,
    pub workspace: Workspace,
    pub resource: Resource,
    pub previous_version: Version,
    pub version: Version,
    pub size: u16,
    pub retry: Retry,
    pub sha256: [u8; 32],
}
impl Operation {
    fn valid(&self) -> bool {
        self.id.lineage() == self.workspace.lineage()
            && self.resource.workspace() == self.workspace
            && self.resource.object() > 4
            && self.resource.object() != self.workspace.root()
            && self.service_instance.lineage() == self.id.lineage()
            && self.id.sequence() == self.version.value()
            && self.previous_version.value() < self.version.value()
            && self.service_instance.sequence() <= self.version.value()
            && self.retry.epoch.value() <= self.version.value()
            && self.size <= 1024
    }
    pub fn encode(self) -> Result<[u8; RECEIPT_BYTES], Error> {
        if !self.valid() {
            return Err(Error::Protocol);
        }
        let mut b = [0; RECEIPT_BYTES];
        b[..16].copy_from_slice(&self.id.lineage());
        b[16..20].copy_from_slice(&self.workspace.root().to_le_bytes());
        b[20..24].copy_from_slice(&self.resource.object().to_le_bytes());
        for (start, value) in [
            (24, self.previous_version.value()),
            (32, self.version.value()),
            (40, self.service_instance.sequence()),
            (48, self.retry.epoch.value()),
            (56, self.retry.key.value()),
        ] {
            b[start..start + 8].copy_from_slice(&value.to_le_bytes());
        }
        b[64..66].copy_from_slice(&self.size.to_le_bytes());
        b[72..].copy_from_slice(&self.sha256);
        Ok(b)
    }
    pub fn decode(b: &[u8; RECEIPT_BYTES]) -> Result<Self, Error> {
        let parse = || {
            let number = |start| u64::from_le_bytes(b[start..start + 8].try_into().unwrap());
            let lineage = b[..16].try_into().unwrap();
            let workspace =
                Workspace::new(lineage, u32::from_le_bytes(b[16..20].try_into().unwrap()))?;
            Ok::<_, Error>(Self {
                id: OperationId::new(lineage, number(32))?,
                service_instance: Instance::new(lineage, number(40))?,
                workspace,
                resource: Resource::new(
                    workspace,
                    u32::from_le_bytes(b[20..24].try_into().unwrap()),
                )?,
                previous_version: Version::new(number(24))?,
                version: Version::new(number(32))?,
                size: u16::from_le_bytes(b[64..66].try_into().unwrap()),
                retry: Retry {
                    epoch: Epoch::new(number(48))?,
                    key: Key::new(number(56))?,
                },
                sha256: b[72..].try_into().unwrap(),
            })
        };
        let value = parse().map_err(|_| Error::Protocol)?;
        if !value.valid() || b[66..72] != [0; 6] {
            return Err(Error::Protocol);
        }
        Ok(value)
    }
    pub fn part(self, op: u8, context: u32, offset: usize) -> Result<Packet, Error> {
        if !matches!(offset, 0 | 40 | 80) {
            return Err(Error::Offset);
        }
        let bytes = self.encode()?;
        let end = (offset + DATA).min(RECEIPT_BYTES);
        let mut p = Packet::new(op);
        p.context = context;
        p.id = offset as u32;
        p.arg = RECEIPT_BYTES as u32;
        p.version = self.id.sequence();
        p.count = (end - offset) as u8;
        p.data[..end - offset].copy_from_slice(&bytes[offset..end]);
        Ok(p)
    }
}
