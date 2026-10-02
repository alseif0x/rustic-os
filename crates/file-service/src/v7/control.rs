// SPDX-License-Identifier: Apache-2.0
//! Owner control between the polls of one V7 storage publication.
//!
//! A tracked commit or admission acceptance, execution or cancellation drives one pollable
//! `Volume7` publication. Between its polls the serving layer's callback gets
//! a [`Control7`]: it may observe the publication and revoke or detach client
//! slots and serve restricted lifecycle requests from captured scope proofs.
//! Issuing grants, maintenance and namespace requests need the whole service
//! and wait until the publication settles.
//!
//! The rules mirror the v5 owner-control loop:
//!
//! - After every callback, expired grants are revoked and the calling client's
//!   authority is checked again. Once it is lost (revoked, detached or
//!   expired), the publication is stopped before its header is submitted: an
//!   outstanding command drains first, and then nothing of it becomes durable.
//!   From header submission on, stopping is too late and the publication
//!   settles; the caller then decides how to report the effect.
//! - Storage consequences of a lost slot (aborting its stage, forgetting its
//!   receipt) need the volume, so they are collected here and applied by the
//!   server once the publication has released it. A slot that lost its grant
//!   cannot reach its stage meanwhile.
//! - Terminal housekeeping (the service's own `AuthorityLost` cancellation)
//!   is driven without a caller: owner control continues between its polls,
//!   but nothing stops it.
use super::grants::Grants;
use super::{
    CLIENTS7, Grant7,
    admission::{active::Active7, scheduling::Scheduler7},
};
use crate::reply;
use core::task::Poll;
use rustic_abi::files::{Error, Packet};
use rustic_fs::{PollDisk7, PollPublication7, Publication7Phase, Volume7, format7::Record7};

/// What the owner may do while a tracked or admission publication is in flight.
pub struct Control7<'a> {
    grants: &'a mut Grants,
    scheduler: &'a mut Scheduler7,
    active: &'a mut Option<Active7>,
    lost: &'a mut u8,
    phase: Publication7Phase,
    pending: bool,
    transfers: u8,
}

impl Control7<'_> {
    /// Route one existing lifecycle-v2 request using the immutable inventory
    /// captured before the current publication borrowed the mounted volume.
    /// The transport supplies the trusted slot and peer binding.
    pub fn request(&mut self, slot: usize, peer: u64, packet: Packet, now: u64) -> Packet {
        self.scheduler
            .request(self.grants, self.active.as_mut(), slot, peer, packet, now)
    }

    /// Phase of the publication in flight. From `Settling` on its header may
    /// have been submitted, and losing the caller's authority no longer stops
    /// it.
    pub fn phase(&self) -> Publication7Phase {
        self.phase
    }

    /// Whether one disk command of the publication is outstanding.
    pub fn pending(&self) -> bool {
        self.pending
    }

    /// Open client candidates that have not lost authority. Their buffers are
    /// released after settlement; the publishing candidate is already consumed.
    pub fn transfer_count(&self) -> usize {
        (self.transfers & !*self.lost).count_ones() as usize
    }

    /// Read-only snapshot for endpoint lifecycle routing.
    pub fn grant_at(&self, slot: usize) -> Option<Grant7> {
        self.grants.grant_at(slot)
    }

    /// Permanently revoke the slot's current generation. A caller in this
    /// slot loses its authority at the next check; the slot's stage and
    /// receipt are dropped once the publication has settled.
    pub fn revoke(&mut self, slot: usize) -> Result<u8, Error> {
        let lost = self.grants.revoke_mask(slot)?;
        *self.lost |= lost;
        Ok(lost)
    }

    /// Fence every current member of a root generation, including when called
    /// from the callback of another slot in that group.
    pub fn revoke_root(&mut self, root: u32) -> u8 {
        let lost = self.grants.revoke_root(root);
        *self.lost |= lost;
        lost
    }

    /// Forget the slot after its endpoint closed, with the same deferred
    /// storage consequences as [`Self::revoke`].
    pub fn detach(&mut self, slot: usize) -> u8 {
        if slot >= CLIENTS7 {
            return 0;
        }
        let lost = self.grants.detach_with_loss(slot);
        *self.lost |= lost;
        lost
    }
}

/// The client whose request started the publication.
#[derive(Clone, Copy)]
pub(super) struct Caller7 {
    pub(super) slot: usize,
    pub(super) peer: u64,
    pub(super) context: u32,
}

/// The serving layer's callback, the caller and the slots that lost their
/// grant during the request.
pub(super) struct Owner<'a> {
    grants: &'a mut Grants,
    scheduler: &'a mut Scheduler7,
    caller: Caller7,
    lost: u8,
    transfers: u8,
    active: Option<Active7>,
    cleanup: bool,
    control: &'a mut dyn FnMut(&mut Control7<'_>) -> u64,
}

/// How a driven publication ended.
pub(super) struct Driven {
    /// The record it made durable (or the retained record a replay reports);
    /// `None` when it was stopped before its header.
    pub(super) record: Option<Record7>,
    /// The first authority failure of the caller observed while it ran.
    pub(super) denied: Option<Error>,
}

impl<'a> Owner<'a> {
    pub(super) fn new(
        grants: &'a mut Grants,
        scheduler: &'a mut Scheduler7,
        caller: Caller7,
        transfers: u8,
        control: &'a mut dyn FnMut(&mut Control7<'_>) -> u64,
    ) -> Self {
        Self {
            grants,
            scheduler,
            caller,
            lost: 0,
            transfers,
            active: None,
            cleanup: false,
            control,
        }
    }

    /// Capture immutable admission scope proofs before `Volume7` is mutably
    /// borrowed by execution/prevention publication.
    pub(super) fn activate(&mut self, volume: &Volume7, record: Record7) -> Result<(), Error> {
        let scope = super::admission::scope::Scope7::capture(volume, self.grants, record)?;
        self.active = Some(Active7::new(scope));
        Ok(())
    }

    pub(super) fn set_cleanup(&mut self, cleanup: bool) {
        self.cleanup = cleanup;
    }

    pub(super) fn scheduled(&self, id: rustic_abi::files::admission::AdmissionId) -> bool {
        self.scheduler.contains(id)
    }

    pub(super) fn stop_active(&mut self) {
        if let Some(active) = self.active.as_mut() {
            active.stop();
        }
    }

    pub(super) fn active_stopping(&self) -> bool {
        self.active.as_ref().is_some_and(Active7::stopping)
    }

    /// Check the original requester's current authority after service-owned
    /// terminal prevention has settled. A stale caller must not receive status.
    pub(super) fn recheck_caller(&mut self, right: u8) -> Result<(), Error> {
        let now = self.poll(Publication7Phase::Committed, false);
        self.expire(now);
        self.caller_holds(now, right).map_err(|_| Error::Uncertain)
    }

    /// Slots that lost their grant while the request ran.
    pub(super) fn lost(&self) -> u8 {
        self.lost
    }

    /// Commit or acceptance consumes its candidate before the first publication poll.
    pub(super) fn consumed(&mut self, slot: usize) {
        self.transfers &= !(1 << slot);
    }

    /// One owner-control opportunity; returns the owner's clock.
    fn poll(&mut self, phase: Publication7Phase, pending: bool) -> u64 {
        if let Some(active) = self.active.as_mut() {
            active.observe(phase, pending, self.cleanup);
        }
        (self.control)(&mut Control7 {
            grants: self.grants,
            scheduler: self.scheduler,
            active: &mut self.active,
            lost: &mut self.lost,
            phase,
            pending,
            transfers: self.transfers,
        })
    }

    /// Revoke every grant that expired by `now`.
    fn expire(&mut self, now: u64) {
        self.lost |= self.grants.expire(now);
    }

    /// The caller's live authority for `right` at `now`.
    fn caller_holds(&self, now: u64, right: u8) -> Result<(), Error> {
        let caller = self.caller;
        self.grants
            .check(caller.slot, caller.peer, caller.context, now)?
            .holds(right)
    }
}

/// Drive `publication` to settlement with owner control between its polls.
///
/// A publication already settled on entry (a replay without I/O) is returned
/// without calling owner control. Otherwise, with `right`, the caller must
/// keep holding it: the first loss is recorded
/// in [`Driven::denied`] and stops the publication if its header was not yet
/// submitted. Without `right` (service housekeeping) nothing stops it.
///
/// A disk failure is [`Error::Uncertain`] and leaves the volume fenced until a
/// remount; a publication that ends cancelled without having been stopped is
/// also `Uncertain`.
pub(super) fn drive<D: PollDisk7>(
    owner: &mut Owner<'_>,
    mut publication: PollPublication7<'_, D>,
    right: Option<u8>,
) -> Result<Driven, Error> {
    // A replay settled without I/O: there is nothing in flight to control,
    // and the caller's authority was checked when the request was admitted.
    if publication.phase() == Publication7Phase::Committed {
        let record = publication.result().ok_or(Error::Uncertain)?;
        return Ok(Driven {
            record: Some(record),
            denied: None,
        });
    }
    let mut denied = None;
    loop {
        let now = owner.poll(publication.phase(), publication.pending());
        if let Some(right) = right
            && denied.is_none()
            && let Err(error) = owner.caller_holds(now, right)
        {
            denied = Some(error);
        }
        // After the check, so an expired caller is told `Expired`.
        owner.expire(now);
        if denied.is_some() || right.is_some() && owner.active_stopping() {
            // Caller authority loss and a live stop both request a pre-header
            // abort. The caller denial remains decisive if both are present.
            // Service-owned prevention uses `right == None` and cannot stop.
            publication.abort_before_header().map_err(reply::error)?;
        }
        match publication.phase() {
            Publication7Phase::Committed => {
                let record = publication.result().ok_or(Error::Uncertain)?;
                return Ok(Driven {
                    record: Some(record),
                    denied,
                });
            }
            Publication7Phase::Cancelled
                if denied.is_some() || right.is_some() && owner.active_stopping() =>
            {
                return Ok(Driven {
                    record: None,
                    denied,
                });
            }
            Publication7Phase::Cancelled => return Err(Error::Uncertain),
            _ => {}
        }
        if let Poll::Ready(Err(error)) = publication.poll_advance() {
            return Err(reply::error(error));
        }
    }
}
