// SPDX-License-Identifier: Apache-2.0
//! Explicitly selected V7 native service over one exclusively borrowed mounted
//! volume: bounded range reads, profile-2 tracked replacement and lookups of
//! retained records.
//!
//! The composing [`Server7`] routes each request after the envelope and grant
//! checks, and keeps the storage consequences of authority changes together:
//! revoking, detaching, expiring or replacing a slot aborts that slot's open
//! stage and forgets its receipt. The v5 `Server` is unaffected.
mod grants;
mod lookup;
mod read;
mod scope;
mod transfer;
mod write;

pub use grants::{CLIENTS7, Grant7, GrantRequest7, READ_ONLY7, TRACKED_WRITE7};

use rustic_abi::files::*;
use rustic_fs::{Disk, Volume7};

pub struct Server7<'a> {
    volume: &'a mut Volume7,
    grants: grants::Grants,
    writes: write::Writes,
}

impl<'a> Server7<'a> {
    pub fn new(volume: &'a mut Volume7) -> Self {
        let writes = write::Writes::new(volume);
        Self {
            volume,
            grants: grants::Grants::new(),
            writes,
        }
    }

    /// The served volume, for read-only inspection by the owner.
    pub fn volume(&self) -> &Volume7 {
        self.volume
    }

    /// Install one scope and return its endpoint/context binding. Replacing a
    /// slot makes its previous context stale and aborts its open stage.
    pub fn grant(&mut self, slot: usize, request: GrantRequest7) -> Result<Grant7, Error> {
        let grant = self.grants.grant(self.volume, slot, request)?;
        self.writes.reset(self.volume, slot);
        Ok(grant)
    }

    /// Permanently revoke the current generation while retaining its endpoint
    /// binding for the serving layer's close/detach bookkeeping. Any open
    /// stage is aborted without I/O, so no later request can commit it.
    pub fn revoke(&mut self, slot: usize) -> Result<(), Error> {
        self.grants.revoke(slot)?;
        self.writes.reset(self.volume, slot);
        Ok(())
    }

    /// Forget the slot after the endpoint has detached. A later request has no
    /// installed authority, and a future grant receives a different context.
    pub fn detach(&mut self, slot: usize) {
        self.grants.detach(slot);
        self.writes.reset(self.volume, slot);
    }

    /// Mark expired slots revoked, abort their stages and return their bit
    /// mask for endpoint cleanup.
    pub fn expire(&mut self, now: u64) -> u8 {
        let expired = self.grants.expire(now);
        for slot in 0..CLIENTS7 {
            if expired & (1 << slot) != 0 {
                self.writes.reset(self.volume, slot);
            }
        }
        expired
    }

    /// Read-only snapshot for endpoint lifecycle routing.
    pub fn grant_at(&self, slot: usize) -> Option<Grant7> {
        self.grants.grant_at(slot)
    }

    pub fn handle(
        &mut self,
        disk: &mut impl Disk,
        slot: usize,
        peer: u64,
        request: Packet,
        now: u64,
    ) -> Packet {
        match self.dispatch(disk, slot, peer, request, now) {
            Ok(reply) => reply,
            Err(error) => {
                let mut response = Packet::new(request.op);
                response.context = request.context;
                response.status = error as u8;
                response
            }
        }
    }

    fn dispatch(
        &mut self,
        disk: &mut impl Disk,
        slot: usize,
        peer: u64,
        packet: Packet,
        now: u64,
    ) -> Result<Packet, Error> {
        crate::validation::envelope(&packet)?;
        let grant = self.grants.check(slot, peer, packet.context, now)?;
        match packet.op {
            REFERENCES | READ_OPEN | READ_CHUNK => {
                crate::validation::request(&packet)?;
                read::request(self.volume, disk, grant, packet)
            }
            _ if write::selected(&packet) => {
                self.writes.request(self.volume, disk, slot, grant, packet)
            }
            _ => Err(Error::Unsupported),
        }
    }
}
