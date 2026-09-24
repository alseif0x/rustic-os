// SPDX-License-Identifier: Apache-2.0
//! Explicitly selected V7 native service over one exclusively borrowed mounted
//! volume: bounded range reads, profile-2 tracked replacement, profile-2 staged
//! admission with explicit execution and cancellation, lookups of retained
//! records and owner-requested retention maintenance.
//!
//! The composing [`Server7`] routes each request after the envelope and grant
//! checks, and keeps the storage consequences of authority changes together:
//! revoking, detaching, expiring or replacing a slot aborts that slot's open
//! stage and forgets its receipt. Maintenance is an owner operation with no
//! client packet: the serving layer calls [`Server7::maintain_retention`] only
//! for its administrative channel. The v5 `Server` is unaffected.
//!
//! Admission publications (acceptance, execution, cancellation) are pollable.
//! [`Server7::handle_with`] drives them over the caller's pollable disk and
//! hands a [`Control7`] to the serving layer between polls, so the owner can
//! revoke or detach clients while one is in flight, with the v5 consequences
//! for the caller's publication (see the `control` module).
//! [`Server7::handle`] settles them synchronously inside the request.
mod admission;
mod control;
mod grants;
mod lookup;
mod read;
mod retention;
mod scope;
mod transfer;
mod write;

pub use control::Control7;
pub use grants::{ADMISSION7, CLIENTS7, Grant7, GrantRequest7, READ_ONLY7, TRACKED_WRITE7};
pub use retention::Maintenance7;

use rustic_abi::files::*;
use rustic_fs::{Disk, PollDisk7, Volume7};

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

    /// Owner-requested retention maintenance: drop every terminal retained
    /// record, free the snapshot sectors no live file owns and publish the
    /// next retry epoch.
    ///
    /// - `Ok`: published; every slot forgets its cached receipt, and retries
    ///   and lookups that name the old epoch answer `ExpiredEpoch`.
    /// - `Busy`: a client transfer, volume stage or unresolved admission is
    ///   open; nothing changed.
    /// - `Exhausted` or `Corrupt` before the publication starts (no epoch or
    ///   sequence left, or current ownership that does not validate): nothing
    ///   changed.
    /// - `Uncertain`: the volume was already fenced, a disk write or flush of
    ///   the publication failed, or the published effect could not be
    ///   described.
    /// - Any error from inside the publication, including `Uncertain` and a
    ///   candidate that does not validate, leaves the volume fenced until a
    ///   remount, which selects whichever generation became durable.
    ///
    /// Whenever the volume ends fenced, after any error, cached receipts are
    /// forgotten too: they may name records the durable generation no longer
    /// holds.
    ///
    /// Only the administrative channel may reach this; no client packet does.
    pub fn maintain_retention(&mut self, disk: &mut impl Disk) -> Result<Maintenance7, Error> {
        let result = retention::maintain(self.volume, disk, self.writes.transfers_open());
        if result.is_ok() || self.volume.header().is_err() {
            self.writes.forget_receipts();
        }
        result
    }

    /// Read-only snapshot for endpoint lifecycle routing.
    pub fn grant_at(&self, slot: usize) -> Option<Grant7> {
        self.grants.grant_at(slot)
    }

    /// Serve one client request, settling any admission publication inside
    /// it with no owner control between its polls.
    pub fn handle(
        &mut self,
        disk: &mut impl Disk,
        slot: usize,
        peer: u64,
        request: Packet,
        now: u64,
    ) -> Packet {
        let mut disk = crate::disk::Synchronous(disk);
        self.handle_with(&mut disk, slot, peer, request, now, |_| now)
    }

    /// Serve one client request, driving an admission publication over the
    /// pollable `disk` and calling `control` before each of its polls. The
    /// callback returns the owner's clock and may revoke or detach slots
    /// through its [`Control7`]; it must not block indefinitely, and it is
    /// never called for requests without a publication.
    ///
    /// If the caller loses its authority before the publication's header is
    /// submitted, nothing of it becomes durable: an execution is then
    /// recorded cancelled with cause `AuthorityLost` by a second, unstoppable
    /// publication, and the reply is the authority error. Later losses let
    /// the publication settle: a new admission is then likewise cancelled
    /// with `AuthorityLost` before the authority error is returned, while a
    /// settled execution or cancellation stands and is reported `Uncertain`.
    /// The caller's endpoint is normally gone by then, so the reply is only
    /// delivered when the binding survived.
    pub fn handle_with<D: Disk + PollDisk7>(
        &mut self,
        disk: &mut D,
        slot: usize,
        peer: u64,
        request: Packet,
        now: u64,
        mut control: impl FnMut(&mut Control7<'_>) -> u64,
    ) -> Packet {
        match self.dispatch(disk, slot, peer, request, now, &mut control) {
            Ok(reply) => reply,
            Err(error) => {
                let mut response = Packet::new(request.op);
                response.context = request.context;
                response.status = error as u8;
                response
            }
        }
    }

    fn dispatch<D: Disk + PollDisk7>(
        &mut self,
        disk: &mut D,
        slot: usize,
        peer: u64,
        packet: Packet,
        now: u64,
        control: &mut dyn FnMut(&mut Control7<'_>) -> u64,
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
            _ if admission::selected(&packet) => {
                let caller = control::Caller7 {
                    slot,
                    peer,
                    context: packet.context,
                };
                let mut owner = control::Owner::new(&mut self.grants, caller, control);
                let result = admission::request(
                    &mut self.writes,
                    self.volume,
                    disk,
                    slot,
                    grant,
                    packet,
                    &mut owner,
                );
                // Slots that lost their grant during the publication drop
                // their stages and receipts now that the volume is free.
                let lost = owner.lost();
                for index in 0..CLIENTS7 {
                    if lost & (1 << index) != 0 {
                        self.writes.reset(self.volume, index);
                    }
                }
                result
            }
            _ => Err(Error::Unsupported),
        }
    }
}
