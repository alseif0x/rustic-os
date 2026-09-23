// SPDX-License-Identifier: Apache-2.0
//! Explicit, read-only native range service for one mounted v7 volume.
use crate::reply;
use rustic_abi::files::{
    read::{Header, MAX_RANGE, Request},
    reference::{Epoch, References, Version},
    *,
};
use rustic_fs::{Disk, Error as FsError, Kind, Volume7, format7::NODES};
use sha2::{Digest, Sha256};

/// Independent read authority slots. This path has no write, inspection or
/// admission rights to attenuate or inherit.
pub const READ_CLIENTS7: usize = 4;

/// Owner-issued endpoint binding for one slot. `context` is a fresh generation
/// returned by [`ReadServer7::grant`]; all other fields are read-only authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadGrant7 {
    pub peer: u64,
    pub endpoint: u64,
    pub context: u32,
    pub scope: u32,
    /// Zero means no expiry; otherwise requests at or after this time are denied.
    pub expires: u64,
}

#[derive(Clone, Copy)]
struct Slot {
    grant: ReadGrant7,
    revoked: bool,
}

/// A separately selected read-only V7 path. It borrows one mounted volume and
/// never changes it; production startup and the existing v5 `Server` are
/// unaffected until a caller explicitly constructs this type.
pub struct ReadServer7<'a> {
    volume: &'a Volume7,
    slots: [Option<Slot>; READ_CLIENTS7],
    next_context: u32,
}

impl<'a> ReadServer7<'a> {
    pub fn new(volume: &'a Volume7) -> Self {
        Self {
            volume,
            slots: [None; READ_CLIENTS7],
            next_context: 1,
        }
    }

    /// Install one direct, read-only scope and return its endpoint/context
    /// binding. Replacing a slot makes its previous context stale.
    pub fn grant(
        &mut self,
        slot: usize,
        peer: u64,
        endpoint: u64,
        scope: u32,
        expires: u64,
    ) -> Result<ReadGrant7, Error> {
        if slot >= READ_CLIENTS7 || peer == 0 || endpoint == 0 || scope == 0 {
            return Err(Error::Invalid);
        }
        let scope_node = match self.volume.stat(scope) {
            Ok(node) => node,
            Err(FsError::NotFound) => return Err(Error::Invalid),
            Err(error) => return Err(reply::error(error)),
        };
        let workspace_root = self.authority_node(4)?;
        if !self.within(scope_node, workspace_root)? {
            return Err(Error::Denied);
        }
        let next = self.next_context.checked_add(1).ok_or(Error::Exhausted)?;
        let grant = ReadGrant7 {
            peer,
            endpoint,
            context: self.next_context,
            scope,
            expires,
        };
        self.next_context = next;
        self.slots[slot] = Some(Slot {
            grant,
            revoked: false,
        });
        Ok(grant)
    }

    /// Permanently revoke the current generation while retaining its endpoint
    /// binding for the serving layer's close/detach bookkeeping.
    pub fn revoke(&mut self, slot: usize) -> Result<(), Error> {
        let entry = self
            .slots
            .get_mut(slot)
            .ok_or(Error::Invalid)?
            .as_mut()
            .ok_or(Error::NotFound)?;
        entry.revoked = true;
        Ok(())
    }

    /// Forget the slot after the endpoint has detached. A later request has no
    /// installed authority, and a future grant receives a different context.
    pub fn detach(&mut self, slot: usize) {
        if let Some(entry) = self.slots.get_mut(slot) {
            *entry = None;
        }
    }

    /// Mark expired slots revoked and return their bit mask for endpoint cleanup.
    pub fn expire(&mut self, now: u64) -> u8 {
        let mut expired = 0;
        for (index, entry) in self.slots.iter_mut().enumerate() {
            if let Some(entry) = entry
                && !entry.revoked
                && entry.grant.expires != 0
                && now >= entry.grant.expires
            {
                entry.revoked = true;
                expired |= 1 << index;
            }
        }
        expired
    }

    /// Read-only snapshot for endpoint lifecycle routing.
    pub fn grant_at(&self, slot: usize) -> Option<ReadGrant7> {
        self.slots
            .get(slot)
            .copied()
            .flatten()
            .map(|entry| entry.grant)
    }

    pub fn handle(
        &self,
        disk: &mut impl Disk,
        slot: usize,
        peer: u64,
        request: Packet,
        now: u64,
    ) -> Packet {
        let mut response = Packet::new(request.op);
        response.context = request.context;
        match self.read_request(disk, slot, peer, request, now) {
            Ok(reply) => reply,
            Err(error) => {
                response.status = error as u8;
                response
            }
        }
    }

    fn read_request(
        &self,
        disk: &mut impl Disk,
        slot: usize,
        peer: u64,
        packet: Packet,
        now: u64,
    ) -> Result<Packet, Error> {
        crate::validation::request(&packet)?;
        let grant = self
            .slots
            .get(slot)
            .copied()
            .flatten()
            .ok_or(Error::Denied)?;
        grant.check(peer, packet.context, now)?;
        if !matches!(packet.op, REFERENCES | READ_OPEN | READ_CHUNK) {
            return Err(Error::Unsupported);
        }

        if packet.op == REFERENCES {
            self.authorized_resource(grant.grant.scope, packet.arg, packet.id)?;
            let lineage = self.volume.header().map_err(reply::error)?.lineage;
            return References::new(lineage, packet.arg, packet.id)?.packet(packet.context);
        }

        let request = Request::decode(&packet)?;
        let node = self.authorized_resource(
            grant.grant.scope,
            request.workspace.root(),
            request.resource.object(),
        )?;
        let header = self.volume.header().map_err(reply::error)?;
        if header.lineage != request.workspace.lineage() {
            return Err(Error::Denied);
        }

        let expected = request.expected_version.map(Version::value);
        if packet.op == READ_OPEN {
            let mut bytes = [0; MAX_RANGE];
            let count = self
                .volume
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
            let count = self
                .volume
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

    fn authorized_resource(
        &self,
        scope: u32,
        workspace: u32,
        resource: u32,
    ) -> Result<rustic_fs::format7::Node7, Error> {
        let workspace_node = self.authority_node(workspace)?;
        if workspace_node.kind != Kind::Directory {
            return Err(Error::Denied);
        }
        let resource_node = self.authority_node(resource)?;
        if !self.within(resource_node, workspace_node)?
            || !self.within(resource_node, self.authority_node(scope)?)?
        {
            return Err(Error::Denied);
        }
        Ok(resource_node)
    }

    fn authority_node(&self, id: u32) -> Result<rustic_fs::format7::Node7, Error> {
        self.volume.stat(id).map_err(|error| match error {
            FsError::NotFound => Error::Denied,
            other => reply::error(other),
        })
    }

    /// Walk only the mounted generation's verified parent links. A file scope
    /// reaches itself; only directories can authorize descendants.
    fn within(
        &self,
        resource: rustic_fs::format7::Node7,
        ancestor: rustic_fs::format7::Node7,
    ) -> Result<bool, Error> {
        if resource.id == ancestor.id {
            return Ok(true);
        }
        if ancestor.kind != Kind::Directory || resource.space != ancestor.space {
            return Ok(false);
        }
        let mut current = resource;
        for _ in 0..NODES {
            if current.parent == 0 {
                return Ok(false);
            }
            let parent = self.authority_node(current.parent)?;
            if parent.kind != Kind::Directory || parent.space != current.space {
                return Ok(false);
            }
            if parent.id == ancestor.id {
                return Ok(true);
            }
            current = parent;
        }
        Ok(false)
    }
}

impl Slot {
    fn check(self, peer: u64, context: u32, now: u64) -> Result<(), Error> {
        if self.grant.peer != peer {
            return Err(Error::Denied);
        }
        if self.revoked || self.grant.context != context {
            return Err(Error::Revoked);
        }
        if self.grant.expires != 0 && now >= self.grant.expires {
            return Err(Error::Expired);
        }
        Ok(())
    }
}
