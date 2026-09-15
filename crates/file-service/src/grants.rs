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
        // A derived level never inherits or introduces a second scope: attenuation is
        // checked against the parent's primary scope only.
        grant.second = 0;
        self.install(slot, grant, parent)
    }

    /// Attach the one additional object scope of an already installed root.
    ///
    /// The caller proves it means the grant it just issued by naming its generation.
    /// Peer, endpoint, rights, subject, deadline, root and generation are untouched;
    /// only the reachable set grows by one file object that is disjoint from the
    /// primary scope. A grant that already has a second scope cannot be extended
    /// again, so this cannot be chained into an unbounded widening.
    /// Only a root may be extended: a derived helper stays attenuated to its parent.
    pub fn extend(&mut self, slot: usize, generation: u32, second: u32) -> Result<u32, Error> {
        if self.clients.roots.get(slot) != Some(&generation) {
            return Err(Error::Denied);
        }
        let grant = self
            .clients
            .grants
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or(Error::NotFound)?;
        if grant.generation != generation || grant.rights == 0 {
            return Err(Error::Revoked);
        }
        if grant.second != 0 || second == 0 {
            return Err(Error::Denied);
        }
        Self::second_scope(&self.volume, grant.scope, second)?;
        grant.second = second;
        Ok(generation)
    }

    /// A second scope must name one live file outside the primary scope. Zero means
    /// none; the volume root (scope zero) already reaches everything, so a grant
    /// without a bounded primary scope may not carry one. A directory is refused
    /// even when it is disjoint: this addition exists to reach a single companion
    /// object, so it never widens authority over a subtree that can grow.
    fn second_scope(volume: &rustic_fs::Volume, scope: u32, second: u32) -> Result<(), Error> {
        if second == 0 {
            return Ok(());
        }
        if scope == 0
            || second == scope
            || volume.stat(second).map(|n| n.kind) != Ok(rustic_fs::Kind::File)
            || volume.within(second, scope)
            || volume.within(scope, second)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }

    fn install(&mut self, slot: usize, mut grant: Grant, root: u32) -> Result<u32, Error> {
        if slot >= CLIENTS
            || grant.peer == 0
            || grant.endpoint == 0
            || grant.rights == 0
            || grant.rights & !(READ_RIGHT | WRITE_RIGHT | INSPECT_RIGHT | CANCEL_RIGHT) != 0
            || (grant.rights & (INSPECT_RIGHT | CANCEL_RIGHT) != 0 && grant.subject == 0)
            || (grant.scope != 0 && self.volume.stat(grant.scope).is_err())
            || Self::second_scope(&self.volume, grant.scope, grant.second).is_err()
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
