// SPDX-License-Identifier: Apache-2.0
//! Authorized stable references and stateless version-pinned range reads.
use crate::{Grant, Server, reply};
use rustic_abi::files::{
    read::{Header, Request},
    reference::{Epoch, References, Version},
    *,
};
use rustic_fs::{Disk, MAX_FILE};
use sha2::{Digest, Sha256};

impl Server {
    pub(super) fn read_request(
        &self,
        disk: &mut impl Disk,
        grant: Grant,
        packet: Packet,
    ) -> Result<Packet, Error> {
        if packet.op == REFERENCES {
            self.authorized_resource(grant, packet.arg, packet.id)?;
            let (lineage, _) = self.read_identity()?;
            return References::new(lineage, packet.arg, packet.id)?.packet(packet.context);
        }
        let request = Request::decode(&packet)?;
        self.authorized_resource(grant, request.workspace.root(), request.resource.object())?;
        let (lineage, epoch) = self.read_identity()?;
        if lineage != request.workspace.lineage() {
            return Err(Error::Denied);
        }
        let offset = usize::try_from(request.offset).map_err(|_| Error::Size)?;
        let expected = request.expected_version.map(|version| version.value());
        if packet.op == READ_OPEN {
            let mut bytes = [0; MAX_FILE];
            let (node, count) = self
                .volume
                .read_versioned(
                    disk,
                    packet.id,
                    expected,
                    offset,
                    &mut bytes[..usize::from(request.length)],
                )
                .map_err(reply::error)?;
            Header {
                id: node.id,
                size: u64::from(node.length),
                version: Version::new(node.version)?,
                range_sha256: Sha256::digest(&bytes[..count]).into(),
                retry_epoch: Epoch::new(epoch)?,
            }
            .packet(packet.context)
        } else {
            let mut result = Packet::new(READ_CHUNK);
            let count = usize::from(request.length).min(DATA);
            let (node, count) = self
                .volume
                .read_versioned(disk, packet.id, expected, offset, &mut result.data[..count])
                .map_err(reply::error)?;
            result.id = node.id;
            result.arg = u32::from(node.length);
            result.version = node.version;
            result.context = packet.context;
            result.count = count as u8;
            Ok(result)
        }
    }

    fn authorized_resource(&self, grant: Grant, workspace: u32, id: u32) -> Result<(), Error> {
        // An opaque ancestor reference does not grant directory listing or widen
        // a file-only scope. Unknown and foreign objects have the same denial.
        grant.access(&self.volume, id, false)?;
        self.volume
            .resolve(workspace, id)
            .map(|_| ())
            .map_err(|_| Error::Denied)
    }

    fn read_identity(&self) -> Result<([u8; 16], u64), Error> {
        // Ordinary read authority may observe identity/epoch, but cannot inspect
        // receipts. A legacy store must be upgraded explicitly by its owner.
        self.volume.recovery_info().map_err(|error| match error {
            rustic_fs::Error::Unsupported => Error::Unavailable,
            other => reply::error(other),
        })
    }
}
