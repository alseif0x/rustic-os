// SPDX-License-Identifier: Apache-2.0
//! Bounded owner-issued roots and one explicitly provisioned helper level.
use crate::{CLIENTS, Grant, Server};
use rustic_abi::files::{Error, INSPECT_RIGHT, READ_RIGHT, WRITE_RIGHT};

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
            .grants
            .iter()
            .position(|g| g.is_some_and(|g| g.generation == parent))
            .ok_or(Error::Revoked)?;
        let source = self.grants[index].unwrap();
        source.check(source.peer, parent, now)?;
        if index == slot
            || self.roots[index] != parent
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
            || grant.rights & !(READ_RIGHT | WRITE_RIGHT | INSPECT_RIGHT) != 0
            || (grant.rights & INSPECT_RIGHT != 0 && grant.subject == 0)
            || (grant.scope != 0 && self.volume.stat(grant.scope).is_err())
        {
            return Err(Error::Invalid);
        }
        let next = self.next.checked_add(1).ok_or(Error::Exhausted)?;
        // Validate first. Replacing a root fences its previous helper before slot reuse.
        self.detach(slot);
        grant.generation = self.next;
        self.next = next;
        self.roots[slot] = if root == 0 { grant.generation } else { root };
        self.grants[slot] = Some(grant);
        Ok(grant.generation)
    }

    /// Fence every member in one serialized service turn. Already admitted disk calls
    /// have returned before this method runs; an uncertain volume still needs recovery.
    /// Returns a slot mask, allowing the transport to discard undelivered old replies.
    pub fn revoke(&mut self, slot: usize) -> Result<u8, Error> {
        self.grant_at(slot).ok_or(Error::NotFound)?;
        let root = self.roots[slot];
        let mut mask = 0;
        for index in 0..CLIENTS {
            if self.roots[index] == root
                && let Some(grant) = self.grants[index].as_mut()
            {
                grant.rights = 0;
                self.transfers.clear(index);
                mask |= 1 << index;
            }
        }
        Ok(mask)
    }

    pub fn detach(&mut self, slot: usize) {
        if let Some(grant) = self.grant_at(slot) {
            if self.roots[slot] == grant.generation {
                let _ = self.revoke(slot);
            }
            self.transfers.clear(slot);
            self.grants[slot] = None;
            self.roots[slot] = 0;
        }
    }
    pub fn grant_at(&self, slot: usize) -> Option<Grant> {
        self.grants.get(slot).copied().flatten()
    }
    pub fn expire(&mut self, now: u64) {
        for slot in 0..CLIENTS {
            if self.grants[slot].is_some_and(|g| g.expires != 0 && now >= g.expires) {
                self.transfers.clear(slot);
            }
        }
    }
    pub fn pending(&self) -> usize {
        self.transfers.count()
    }
}
