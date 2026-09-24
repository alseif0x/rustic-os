// SPDX-License-Identifier: Apache-2.0
//! Bounded native range reads and references over the mounted V7 volume.
//! Reads never change the volume.
use super::{Grant7, scope};
use crate::reply;
use rustic_abi::files::{
    read::{Header, MAX_RANGE, Request},
    reference::{Epoch, References, Version},
    *,
};
use rustic_fs::{Disk, Volume7};
use sha2::{Digest, Sha256};

/// Serve one already envelope-, grant- and shape-checked read request.
pub(super) fn request(
    volume: &Volume7,
    disk: &mut impl Disk,
    grant: Grant7,
    packet: Packet,
) -> Result<Packet, Error> {
    grant.holds(READ_RIGHT)?;
    if packet.op == REFERENCES {
        scope::authorized_resource(volume, grant.scope, packet.arg, packet.id)?;
        let lineage = volume.header().map_err(reply::error)?.lineage;
        return References::new(lineage, packet.arg, packet.id)?.packet(packet.context);
    }

    let request = Request::decode(&packet)?;
    let node = scope::authorized_resource(
        volume,
        grant.scope,
        request.workspace.root(),
        request.resource.object(),
    )?;
    let header = volume.header().map_err(reply::error)?;
    if header.lineage != request.workspace.lineage() {
        return Err(Error::Denied);
    }

    let expected = request.expected_version.map(Version::value);
    if packet.op == READ_OPEN {
        let mut bytes = [0; MAX_RANGE];
        let count = volume
            .read_range(
                disk,
                node.id,
                expected,
                request.offset,
                &mut bytes[..usize::from(request.length)],
            )
            .map_err(reply::error)?;
        Header {
            id: node.id,
            size: u64::from(node.length),
            version: Version::new(node.version)?,
            range_sha256: Sha256::digest(&bytes[..count]).into(),
            retry_epoch: Epoch::new(header.epoch)?,
        }
        .packet(packet.context)
    } else {
        let mut result = Packet::new(READ_CHUNK);
        let length = usize::from(request.length).min(DATA);
        let count = volume
            .read_range(
                disk,
                node.id,
                expected,
                request.offset,
                &mut result.data[..length],
            )
            .map_err(reply::error)?;
        result.id = node.id;
        result.arg = node.length;
        result.version = node.version;
        result.context = packet.context;
        result.count = count as u8;
        Ok(result)
    }
}
