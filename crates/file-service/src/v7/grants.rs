// SPDX-License-Identifier: Apache-2.0
//! Owner-issued V7 endpoint authority: slot table, rights profiles and checks.
//!
//! The table only records what the owner installed. Storage effects of losing a
//! slot (aborting a stage, forgetting a receipt) belong to the composing server.
use rustic_abi::files::{Error, INSPECT_RIGHT, READ_RIGHT, WRITE_RIGHT};
use rustic_fs::Volume7;

/// Independent client authority slots.
pub const CLIENTS7: usize = 4;

/// Read-only profile: bounded range reads and references.
pub const READ_ONLY7: u8 = READ_RIGHT;
/// Tracked-write profile: reads plus profile-2 tracked replacement and
/// inspection of the receipts this slot produced.
pub const TRACKED_WRITE7: u8 = READ_RIGHT | WRITE_RIGHT | INSPECT_RIGHT;

/// Authority the owner asks to install in one slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrantRequest7 {
    pub peer: u64,
    pub endpoint: u64,
    /// Directory or file below the workspaces root that bounds every request.
    pub scope: u32,
    /// Exactly [`READ_ONLY7`] or [`TRACKED_WRITE7`].
    pub rights: u8,
    /// Retry subject persisted in tracked records. Zero for read-only grants,
    /// nonzero for tracked writes.
    pub subject: u64,
    /// Zero means no expiry; otherwise requests at or after this time are denied.
    pub expires: u64,
}

/// Installed endpoint binding for one slot. `context` is a fresh generation;
/// every other field is the authority the owner requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grant7 {
    pub peer: u64,
    pub endpoint: u64,
    pub context: u32,
    pub scope: u32,
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
    revoked: bool,
}

pub(super) struct Grants {
    slots: [Option<Slot>; CLIENTS7],
    next_context: u32,
}

impl Grants {
    pub(super) const fn new() -> Self {
        Self {
            slots: [None; CLIENTS7],
            next_context: 1,
        }
    }

    /// Install `request` in `slot`, replacing any previous generation.
    pub(super) fn grant(
        &mut self,
        volume: &Volume7,
        slot: usize,
        request: GrantRequest7,
    ) -> Result<Grant7, Error> {
        let subject_valid = match request.rights {
            READ_ONLY7 => request.subject == 0,
            TRACKED_WRITE7 => request.subject != 0,
            _ => false,
        };
        if slot >= CLIENTS7
            || request.peer == 0
            || request.endpoint == 0
            || request.scope == 0
            || !subject_valid
        {
            return Err(Error::Invalid);
        }
        super::scope::grantable(volume, request.scope)?;
        let next = self.next_context.checked_add(1).ok_or(Error::Exhausted)?;
        let grant = Grant7 {
            peer: request.peer,
            endpoint: request.endpoint,
            context: self.next_context,
            scope: request.scope,
            rights: request.rights,
            subject: request.subject,
            expires: request.expires,
        };
        self.next_context = next;
        self.slots[slot] = Some(Slot {
            grant,
            revoked: false,
        });
        Ok(grant)
    }

    pub(super) fn revoke(&mut self, slot: usize) -> Result<(), Error> {
        let entry = self
            .slots
            .get_mut(slot)
            .ok_or(Error::Invalid)?
            .as_mut()
            .ok_or(Error::NotFound)?;
        entry.revoked = true;
        Ok(())
    }

    pub(super) fn detach(&mut self, slot: usize) {
        if let Some(entry) = self.slots.get_mut(slot) {
            *entry = None;
        }
    }

    /// Mark expired slots revoked and return their bit mask.
    pub(super) fn expire(&mut self, now: u64) -> u8 {
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
        if entry.revoked || entry.grant.context != context {
            return Err(Error::Revoked);
        }
        if entry.grant.expires != 0 && now >= entry.grant.expires {
            return Err(Error::Expired);
        }
        Ok(entry.grant)
    }
}
