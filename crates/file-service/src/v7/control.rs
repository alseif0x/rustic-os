// SPDX-License-Identifier: Apache-2.0
//! Owner control between the polls of one V7 admission publication.
//!
//! An admission acceptance, execution or cancellation drives one pollable
//! `Volume7` publication. Between its polls the serving layer's callback gets
//! a [`Control7`]: it may observe the publication and revoke or detach client
//! slots, which is all the owner can do while the volume is borrowed. Issuing
//! grants, maintenance and every client request need the whole service and
//! wait until the publication settles.
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
use super::{CLIENTS7, Grant7};
use crate::reply;
use core::task::Poll;
use rustic_abi::files::Error;
use rustic_fs::{PollDisk7, PollPublication7, Publication7Phase, format7::Record7};

/// What the owner may do while an admission publication is in flight.
pub struct Control7<'a> {
    grants: &'a mut Grants,
    lost: &'a mut u8,
    phase: Publication7Phase,
    pending: bool,
}

impl Control7<'_> {
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

    /// Read-only snapshot for endpoint lifecycle routing.
    pub fn grant_at(&self, slot: usize) -> Option<Grant7> {
        self.grants.grant_at(slot)
    }

    /// Permanently revoke the slot's current generation. A caller in this
    /// slot loses its authority at the next check; the slot's stage and
    /// receipt are dropped once the publication has settled.
    pub fn revoke(&mut self, slot: usize) -> Result<(), Error> {
        self.grants.revoke(slot)?;
        *self.lost |= 1 << slot;
        Ok(())
    }

    /// Forget the slot after its endpoint closed, with the same deferred
    /// storage consequences as [`Self::revoke`].
    pub fn detach(&mut self, slot: usize) {
        if slot < CLIENTS7 {
            self.grants.detach(slot);
            *self.lost |= 1 << slot;
        }
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
    caller: Caller7,
    lost: u8,
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
        caller: Caller7,
        control: &'a mut dyn FnMut(&mut Control7<'_>) -> u64,
    ) -> Self {
        Self {
            grants,
            caller,
            lost: 0,
            control,
        }
    }

    /// Slots that lost their grant while the request ran.
    pub(super) fn lost(&self) -> u8 {
        self.lost
    }

    /// One owner-control opportunity; returns the owner's clock.
    fn poll(&mut self, phase: Publication7Phase, pending: bool) -> u64 {
        (self.control)(&mut Control7 {
            grants: self.grants,
            lost: &mut self.lost,
            phase,
            pending,
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
        if denied.is_some() {
            // Cancelled, draining or too late: settlement decides which.
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
            Publication7Phase::Cancelled if denied.is_some() => {
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
