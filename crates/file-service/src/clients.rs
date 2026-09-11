// SPDX-License-Identifier: Apache-2.0
//! Volatile client authority and staging, independently owned from filesystem I/O.
use crate::{CLIENTS, Grant, transfer::Transfers};
use rustic_abi::files::Error;
pub struct Clients {
    pub(super) grants: [Option<Grant>; CLIENTS],
    pub(super) roots: [u32; CLIENTS],
    pub(super) next: u32,
    pub(super) transfers: Transfers,
}
impl Clients {
    pub(super) fn new() -> Self {
        Self {
            grants: [None; CLIENTS],
            roots: [0; CLIENTS],
            next: 1,
            transfers: Transfers::new(),
        }
    }
    /// Fence future admission immediately. Outstanding I/O must still settle;
    /// this method alone makes no claim about prior file effects.
    /// Returns a slot mask, allowing the transport to discard undelivered old replies.
    pub fn revoke(&mut self, slot: usize) -> Result<u8, Error> {
        self.grant_at(slot).ok_or(Error::NotFound)?;
        Ok(self.revoke_root(self.roots[slot]))
    }

    /// Idempotent fencing by incarnation-local root, including already detached slots.
    pub fn revoke_root(&mut self, root: u32) -> u8 {
        if root == 0 {
            return 0;
        }
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
        mask
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
