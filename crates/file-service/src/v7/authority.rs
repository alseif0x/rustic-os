// SPDX-License-Identifier: Apache-2.0
//! Public owner authority operations and their storage-state consequences.
//!
//! Grant validation and generation relationships live in [`super::grants`].
//! This layer applies every returned loss mask to the stages, plain candidates
//! and receipts owned by `Server7`.
use super::{CLIENTS7, Grant7, GrantRequest7, Server7};
use rustic_abi::files::Error;

impl Server7<'_> {
    /// Install a trusted owner-issued root grant. Replacing an existing root
    /// fences its helper group before the new generation becomes visible.
    pub fn grant(&mut self, slot: usize, request: GrantRequest7) -> Result<Grant7, Error> {
        let (grant, lost) = self.grants.grant_with_loss(self.volume, slot, request)?;
        self.reset_lost(lost);
        Ok(grant)
    }

    /// Derive one attenuated helper directly from a live root generation.
    pub fn derive(
        &mut self,
        slot: usize,
        parent: u32,
        request: GrantRequest7,
        now: u64,
    ) -> Result<Grant7, Error> {
        let (grant, lost) =
            self.grants
                .derive_with_loss(self.volume, slot, parent, request, now)?;
        self.reset_lost(lost);
        Ok(grant)
    }

    /// Attach the root's one optional, disjoint live file scope.
    pub fn extend(&mut self, slot: usize, generation: u32, second: u32) -> Result<u32, Error> {
        self.grants.extend(self.volume, slot, generation, second)
    }

    /// Revoke the root group containing `slot` and reset every affected slot.
    pub fn revoke(&mut self, slot: usize) -> Result<u8, Error> {
        let lost = self.grants.revoke_mask(slot)?;
        self.reset_lost(lost);
        Ok(lost)
    }

    /// Revoke one incarnation-local root group. A stale generation returns 0.
    pub fn revoke_root(&mut self, root: u32) -> u8 {
        let lost = self.grants.revoke_root(root);
        self.reset_lost(lost);
        lost
    }

    /// Detach a slot, fencing its root group when the slot owns that root.
    pub fn detach(&mut self, slot: usize) -> u8 {
        let lost = self.grants.detach_with_loss(slot);
        self.reset_lost(lost);
        lost
    }

    /// Expire grants whose deadlines have passed and release their state.
    pub fn expire(&mut self, now: u64) -> u8 {
        let lost = self.grants.expire(now);
        self.reset_lost(lost);
        lost
    }

    /// Read-only snapshot for endpoint lifecycle routing.
    pub fn grant_at(&self, slot: usize) -> Option<Grant7> {
        self.grants.grant_at(slot)
    }

    /// Apply the slot consequences after an in-flight owner-control callback.
    pub(super) fn reset_lost(&mut self, lost: u8) {
        for slot in 0..CLIENTS7 {
            if lost & (1 << slot) != 0 {
                self.writes.reset(self.volume, slot);
                self.plain.reset(slot);
            }
        }
    }
}
