// SPDX-License-Identifier: Apache-2.0
//! Existing namespace queries and bounded legacy reads over native V7 nodes.
//!
//! Authorization is checked against live parent links before returning node
//! metadata or touching file payloads. The 40-byte node response intentionally
//! keeps the existing service-v1 shape while carrying V7's full `u32` length.
use super::{Grant7, scope};
use crate::reply;
use rustic_abi::files::{Error, LIST, LOOKUP, Packet, READ_RIGHT, STAT};
use rustic_fs::{Volume7, format7::Node7};

/// Serve one shape-checked namespace or legacy range-read request.
pub(super) fn request(volume: &Volume7, grant: Grant7, packet: Packet) -> Result<Packet, Error> {
    grant.holds(READ_RIGHT)?;
    match packet.op {
        LOOKUP => lookup(volume, grant, packet),
        STAT => stat(volume, grant, packet),
        LIST => list(volume, grant, packet),
        _ => Err(Error::Unsupported),
    }
}

fn lookup(volume: &Volume7, grant: Grant7, packet: Packet) -> Result<Packet, Error> {
    let node = if packet.id == 0 {
        // The virtual root contains the canonical mounted roots only. A whole-
        // volume grant can see all four; a workspaces grant sees that root alone.
        if grant.scope != 0 && grant.scope != scope::WORKSPACES_ROOT {
            return Err(Error::Denied);
        }
        let node = match volume.lookup(0, packet.payload()) {
            Ok(node) => node,
            Err(rustic_fs::Error::NotFound) if grant.scope == scope::WORKSPACES_ROOT => {
                return Err(Error::Denied);
            }
            Err(error) => return Err(reply::error(error)),
        };
        if !scope::root_visible(grant, node) {
            return Err(Error::Denied);
        }
        node
    } else {
        scope::authorized_node(volume, grant, packet.id)?;
        let node = volume
            .lookup(packet.id, packet.payload())
            .map_err(reply::error)?;
        scope::authorized_node(volume, grant, node.id)?
    };
    Ok(node_reply(packet, node, 0))
}

fn stat(volume: &Volume7, grant: Grant7, packet: Packet) -> Result<Packet, Error> {
    let node = scope::authorized_node(volume, grant, packet.id)?;
    Ok(node_reply(packet, node, 0))
}

fn list(volume: &Volume7, grant: Grant7, packet: Packet) -> Result<Packet, Error> {
    let cursor = u8::try_from(packet.arg).map_err(|_| Error::Invalid)?;
    if packet.id == 0 {
        if grant.scope != 0 && grant.scope != scope::WORKSPACES_ROOT {
            return Err(Error::Denied);
        }
        let total = if grant.scope == 0 { 4 } else { 1 };
        if usize::from(cursor) >= total {
            return Ok(empty_reply(packet));
        }
        let id = if grant.scope == 0 {
            u32::from(cursor) + 1
        } else {
            scope::WORKSPACES_ROOT
        };
        let node = volume.stat(id).map_err(reply::error)?;
        if !scope::root_visible(grant, node) {
            return Err(Error::Corrupt);
        }
        return Ok(node_reply(packet, node, cursor + 1));
    }

    // A cursor is an ordinal among authorized children, never a physical V7
    // node-table slot. Keeping the physical cursor as `usize` also preserves
    // the final slot's successor value of 256 without narrowing or wrapping.
    scope::authorized_node(volume, grant, packet.id)?;
    let mut physical_cursor = 0usize;
    let mut ordinal = 0u16;
    loop {
        let Some((next_physical, node)) = volume
            .list(packet.id, physical_cursor)
            .map_err(reply::error)?
        else {
            return Ok(empty_reply(packet));
        };
        physical_cursor = next_physical;
        match scope::authorized_node(volume, grant, node.id) {
            Ok(_) => {
                if ordinal == u16::from(cursor) {
                    let next_ordinal = ordinal.checked_add(1).ok_or(Error::Corrupt)?;
                    let next_cursor = u8::try_from(next_ordinal).map_err(|_| Error::Corrupt)?;
                    return Ok(node_reply(packet, node, next_cursor));
                }
                ordinal = ordinal.checked_add(1).ok_or(Error::Corrupt)?;
            }
            Err(Error::Denied) => {}
            Err(error) => return Err(error),
        }
    }
}

/// Encode V7 metadata in the existing node reply layout without constructing
/// a legacy `Node`, whose `u16` length cannot represent every V7 file.
pub(super) fn node_reply(packet: Packet, node: Node7, cursor: u8) -> Packet {
    let mut response = Packet::new(packet.op);
    response.context = packet.context;
    response.id = node.id;
    response.arg = node.length;
    response.version = node.version;
    response.count = 40;
    response.data[0] = node.kind as u8;
    response.data[1] = node.space;
    response.data[2] = node.name_length;
    response.data[3] = cursor;
    response.data[4..8].copy_from_slice(&node.parent.to_le_bytes());
    response.data[8..8 + usize::from(node.name_length)].copy_from_slice(node.name());
    response
}

fn empty_reply(packet: Packet) -> Packet {
    let mut response = Packet::new(packet.op);
    response.context = packet.context;
    response
}
