// SPDX-License-Identifier: Apache-2.0
use super::{MAX_FILE_BYTES, PROFILE, RECEIPT_BYTES};
use crate::files::{
    Error, Packet,
    operation::{self, Instance, OperationId, Retry},
    reference::{Resource, Version, Workspace},
};

/// Completed operation; the retained content digest is bound to a 32-bit size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operation {
    pub id: OperationId,
    pub service_instance: Instance,
    pub workspace: Workspace,
    pub resource: Resource,
    pub previous_version: Version,
    pub version: Version,
    pub size: u32,
    pub retry: Retry,
    pub sha256: [u8; 32],
}
impl Operation {
    // The existing codec owns the unchanged identity/version invariants. Size
    // zero is only an internal template; every public encoding writes u32 size
    // and the nonzero profile marker before returning bytes.
    fn identity(&self) -> operation::Operation {
        operation::Operation {
            id: self.id,
            service_instance: self.service_instance,
            workspace: self.workspace,
            resource: self.resource,
            previous_version: self.previous_version,
            version: self.version,
            size: 0,
            retry: self.retry,
            sha256: self.sha256,
        }
    }
    pub fn encode(self) -> Result<[u8; RECEIPT_BYTES], Error> {
        if self.size > MAX_FILE_BYTES {
            return Err(Error::Protocol);
        }
        let mut bytes = self.identity().encode()?;
        bytes[64..68].copy_from_slice(&self.size.to_le_bytes());
        bytes[68..72].copy_from_slice(&PROFILE.to_le_bytes());
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8; RECEIPT_BYTES]) -> Result<Self, Error> {
        let size = u32::from_le_bytes(bytes[64..68].try_into().unwrap());
        if size > MAX_FILE_BYTES || bytes[68..72] != PROFILE.to_le_bytes() {
            return Err(Error::Protocol);
        }
        let mut identity = *bytes;
        identity[64..72].fill(0);
        let old = operation::Operation::decode(&identity)?;
        Ok(Self {
            id: old.id,
            service_instance: old.service_instance,
            workspace: old.workspace,
            resource: old.resource,
            previous_version: old.previous_version,
            version: old.version,
            size,
            retry: old.retry,
            sha256: old.sha256,
        })
    }
    pub fn part(self, op: u8, context: u32, offset: usize) -> Result<Packet, Error> {
        // Preserve the existing fragment envelope and replace only its content.
        let mut p = self.identity().part(op, context, offset)?;
        let bytes = self.encode()?;
        let end = offset + p.count as usize;
        p.data[..p.count as usize].copy_from_slice(&bytes[offset..end]);
        Ok(p)
    }
}
