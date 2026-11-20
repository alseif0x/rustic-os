// SPDX-License-Identifier: Apache-2.0
//! Owner-issued V7 authority: live scopes, rights, root generations and one
//! explicitly provisioned helper level.
//!
//! The table records authority only. The composing server applies the returned
//! slot masks to V7 transfer, stage and receipt state after each owner action.
use super::scope;
use rustic_abi::files::{CANCEL_RIGHT, Error, INSPECT_RIGHT, READ_RIGHT, WRITE_RIGHT};
use rustic_fs::Volume7;

/// Independent client authority slots.
pub const CLIENTS7: usize = 4;

/// Read right bit, also the rights value used by a read-only grant.
pub const READ_ONLY7: u8 = READ_RIGHT;
/// Read/write/inspect rights used by the tracked-write terminal.
pub const TRACKED_WRITE7: u8 = READ_RIGHT | WRITE_RIGHT | INSPECT_RIGHT;
/// Tracked-write rights plus cancellation authority.
pub const ADMISSION7: u8 = TRACKED_WRITE7 | CANCEL_RIGHT;

const KNOWN_RIGHTS: u8 = READ_RIGHT | WRITE_RIGHT | INSPECT_RIGHT | CANCEL_RIGHT;

/// Authority the owner asks to install in one slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrantRequest7 {
    pub peer: u64,
    pub endpoint: u64,
    /// Zero reaches the mounted volume; any other live node bounds a subtree.
    pub scope: u32,
    /// Any nonzero subset of the existing file-service rights.
    pub rights: u8,
    /// Trusted recovery identity. Inspection and cancellation require nonzero.
    pub subject: u64,
    /// Zero means no expiry; otherwise requests at or after this time are denied.
    pub expires: u64,
}

/// Installed endpoint binding for one slot. `context` is a fresh generation;
/// `second` is a root-only optional companion file scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grant7 {
    pub peer: u64,
    pub endpoint: u64,
    pub context: u32,
    pub scope: u32,
    pub second: u32,
    pub rights: u8,
    pub subject: u64,
    pub expires: u64,
}

impl Grant7 {
    /// Whether this grant holds every right in `rights`.
    pub(super) fn holds(self, rights: u8) -> Result<(), Error> {
        if self.rights & rights == rights {
            Ok(())
        } else {
            Err(Error::Denied)
        }
    }
}

#[derive(Clone, Copy)]
struct Slot {
    grant: Grant7,
    expired: bool,
}

pub(super) struct Grants {
    slots: [Option<Slot>; CLIENTS7],
    /// Root generation for each slot, or zero when detached.
    roots: [u32; CLIENTS7],
    next_context: u32,
}

impl Grants {
    pub(super) const fn new() -> Self {
        Self {
            slots: [None; CLIENTS7],
            roots: [0; CLIENTS7],
            next_context: 1,
        }
    }

    /// Install a root grant and report every old slot fenced by replacement.
    pub(super) fn grant_with_loss(
        &mut self,
        volume: &Volume7,
        slot: usize,
        request: GrantRequest7,
    ) -> Result<(Grant7, u8), Error> {
        self.install(volume, slot, request, None)
    }

    /// Derive one helper directly from a live root. Helpers do not delegate and
    /// do not inherit the root's second scope.
    pub(super) fn derive_with_loss(
        &mut self,
        volume: &Volume7,
        slot: usize,
        parent: u32,
        mut request: GrantRequest7,
        now: u64,
    ) -> Result<(Grant7, u8), Error> {
        let parent_slot = self
            .slots
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.grant.context == parent))
            .ok_or(Error::Revoked)?;
        let source = self.slots[parent_slot].ok_or(Error::Revoked)?.grant;
        self.check(parent_slot, source.peer, parent, now)?;
        if parent_slot == slot
            || self.roots[parent_slot] != parent
            || request.rights & !source.rights != 0
            || !scope::scope_within(volume, request.scope, source.scope)
            || source.expires != 0 && (request.expires == 0 || request.expires > source.expires)
        {
            return Err(Error::Denied);
        }
        request.subject = source.subject;
        self.install(volume, slot, request, Some(parent))
    }

    /// Extend a root's reachable set once with one disjoint live file.
    pub(super) fn extend(
        &mut self,
        volume: &Volume7,
        slot: usize,
        generation: u32,
        second: u32,
    ) -> Result<u32, Error> {
        if self.roots.get(slot) != Some(&generation) {
            return Err(Error::Denied);
        }
        let entry = self
            .slots
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or(Error::NotFound)?;
        if entry.grant.context != generation || entry.grant.rights == 0 {
            return Err(Error::Revoked);
        }
        if entry.expired {
            return Err(Error::Expired);
        }
        if entry.grant.second != 0 || second == 0 {
            return Err(Error::Denied);
        }
        scope::validate_second(volume, entry.grant.scope, second)?;
        entry.grant.second = second;
        Ok(generation)
    }

    fn install(
        &mut self,
        volume: &Volume7,
        slot: usize,
        request: GrantRequest7,
        root: Option<u32>,
    ) -> Result<(Grant7, u8), Error> {
        if slot >= CLIENTS7
            || request.peer == 0
            || request.endpoint == 0
            || request.rights == 0
            || request.rights & !KNOWN_RIGHTS != 0
            || request.rights & (INSPECT_RIGHT | CANCEL_RIGHT) != 0 && request.subject == 0
        {
            return Err(Error::Invalid);
        }
        scope::grantable(volume, request.scope)?;
        let generation = self.next_context;
        let next_context = generation.checked_add(1).ok_or(Error::Exhausted)?;
        let grant = Grant7 {
            peer: request.peer,
            endpoint: request.endpoint,
            context: generation,
            scope: request.scope,
            second: 0,
            rights: request.rights,
            subject: request.subject,
            expires: request.expires,
        };

        // Validate the new binding before fencing the current root or helper.
        let lost = self.detach_with_loss(slot);
        self.next_context = next_context;
        self.slots[slot] = Some(Slot {
            grant,
            expired: false,
        });
        self.roots[slot] = root.unwrap_or(generation);
        Ok((grant, lost))
    }

    /// Revoke the root group containing `slot` and return every affected slot.
    pub(super) fn revoke_mask(&mut self, slot: usize) -> Result<u8, Error> {
        self.slots
            .get(slot)
            .and_then(Option::as_ref)
            .ok_or(Error::NotFound)?;
        let root = self.roots[slot];
        Ok(self.revoke_root(root))
    }

    /// Idempotently fence the live members of this incarnation-local root.
    pub(super) fn revoke_root(&mut self, root: u32) -> u8 {
        if root == 0 {
            return 0;
        }
        let mut lost = 0;
        for (index, entry) in self.slots.iter_mut().enumerate() {
            if self.roots[index] == root
                && let Some(entry) = entry.as_mut()
            {
                entry.grant.rights = 0;
                lost |= 1 << index;
            }
        }
        lost
    }

    /// Forget one slot. Detaching its root fences the whole group first;
    /// detaching a helper leaves its root and siblings live.
    pub(super) fn detach_with_loss(&mut self, slot: usize) -> u8 {
        if slot >= CLIENTS7 {
            return 0;
        }
        let Some(entry) = self.slots[slot] else {
            return 0;
        };
        let root = self.roots[slot];
        let mut lost = 1 << slot;
        if root == entry.grant.context {
            lost |= self.revoke_root(root);
        }
        self.slots[slot] = None;
        self.roots[slot] = 0;
        lost
    }

    pub(super) fn expire(&mut self, now: u64) -> u8 {
        let mut lost = 0;
        for (index, entry) in self.slots.iter_mut().enumerate() {
            if let Some(entry) = entry.as_mut()
                && !entry.expired
                && entry.grant.rights != 0
                && entry.grant.expires != 0
                && now >= entry.grant.expires
            {
                entry.expired = true;
                lost |= 1 << index;
            }
        }
        lost
    }

    pub(super) fn grant_at(&self, slot: usize) -> Option<Grant7> {
        self.slots
            .get(slot)
            .copied()
            .flatten()
            .map(|entry| entry.grant)
    }

    /// The live grant for a request from `peer` carrying `context` at `now`.
    pub(super) fn check(
        &self,
        slot: usize,
        peer: u64,
        context: u32,
        now: u64,
    ) -> Result<Grant7, Error> {
        let entry = self
            .slots
            .get(slot)
            .copied()
            .flatten()
            .ok_or(Error::Denied)?;
        if entry.grant.peer != peer {
            return Err(Error::Denied);
        }
        if entry.grant.rights == 0 || entry.grant.context != context {
            return Err(Error::Revoked);
        }
        if entry.expired || entry.grant.expires != 0 && now >= entry.grant.expires {
            return Err(Error::Expired);
        }
        Ok(entry.grant)
    }
}
