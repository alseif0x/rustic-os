// SPDX-License-Identifier: Apache-2.0
//! Bounded owner-issued roots and one explicitly provisioned helper level.
use crate::{CLIENTS, Grant, Server};
use rustic_abi::files::{CANCEL_RIGHT, Error, INSPECT_RIGHT, READ_RIGHT, WRITE_RIGHT};

impl Server {
    /// Only an authenticated administrator may issue a fresh root.
    pub fn grant(&mut self, slot: usize, grant: Grant) -> Result<u32, Error> {
        self.install(slot, grant, 0)
    }

    /// A helper retains its root even if the endpoint moves. No recursive issuance.
    /// Scope, rights, deadline and recovery identity cannot exceed the live parent.
    pub fn derive(
        &mut self,
        slot: usize,
        parent: u32,
        mut grant: Grant,
        now: u64,
    ) -> Result<u32, Error> {
        let index = self
            .clients
            .grants
            .iter()
            .position(|g| g.is_some_and(|g| g.generation == parent))
            .ok_or(Error::Revoked)?;
        let source = self.clients.grants[index].unwrap();
        source.check(source.peer, parent, now)?;
        if index == slot
            || self.clients.roots[index] != parent
            || grant.rights & !source.rights != 0
            || !(source.scope == 0 || self.volume.within(grant.scope, source.scope))
            || source.expires != 0 && (grant.expires == 0 || grant.expires > source.expires)
        {
            return Err(Error::Denied);
        }
        grant.subject = source.subject;
        self.install(slot, grant, parent)
    }

    fn install(&mut self, slot: usize, mut grant: Grant, root: u32) -> Result<u32, Error> {
        if slot >= CLIENTS
            || grant.peer == 0
            || grant.endpoint == 0
            || grant.rights == 0
            || grant.rights & !(READ_RIGHT | WRITE_RIGHT | INSPECT_RIGHT | CANCEL_RIGHT) != 0
            || (grant.rights & (INSPECT_RIGHT | CANCEL_RIGHT) != 0 && grant.subject == 0)
            || (grant.scope != 0 && self.volume.stat(grant.scope).is_err())
        {
            return Err(Error::Invalid);
        }
        let next = self.clients.next.checked_add(1).ok_or(Error::Exhausted)?;
        // Validate first. Replacing a root fences its previous helper before slot reuse.
        self.detach(slot);
        grant.generation = self.clients.next;
        self.clients.next = next;
        self.clients.roots[slot] = if root == 0 { grant.generation } else { root };
        self.clients.grants[slot] = Some(grant);
        Ok(grant.generation)
    }

    pub fn revoke(&mut self, slot: usize) -> Result<u8, Error> {
        self.clients.revoke(slot)
    }
    pub fn revoke_root(&mut self, root: u32) -> u8 {
        self.clients.revoke_root(root)
    }
    pub fn detach(&mut self, slot: usize) {
        self.clients.detach(slot);
    }
    pub fn grant_at(&self, slot: usize) -> Option<Grant> {
        self.clients.grant_at(slot)
    }
    pub fn expire(&mut self, now: u64) {
        self.clients.expire(now);
    }
    pub fn pending(&self) -> usize {
        self.clients.pending()
    }
}
