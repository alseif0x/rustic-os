// SPDX-License-Identifier: Apache-2.0
//! Explicitly selected V7 native service over one exclusively borrowed mounted
//! volume: bounded reads, plain replacement, existing profile-1 and profile-2
//! tracked replacement and staged admission with explicit execution and cancellation,
//! lookups of retained records and owner-requested retention maintenance.
//!
//! The composing [`Server7`] routes each request after the envelope and grant
//! checks, and keeps the storage consequences of authority changes together:
//! revoking, detaching, expiring or replacing a slot aborts that slot's open
//! stage and forgets its receipt. Maintenance is an owner operation with no
//! client packet: the serving layer calls [`Server7::maintain_retention`] only
//! for its administrative channel. The v5 `Server` is unaffected.
//!
//! Tracked commits and admission publications are pollable.
//! [`Server7::handle_with`] drives them over the caller's pollable disk and
//! hands a [`Control7`] to the serving layer between polls, so the owner can
//! revoke or detach clients while one is in flight, with the v5 consequences
//! for the caller's publication (see the `control` module).
//! [`Server7::handle`] settles them synchronously inside the request.
mod admission;
mod authority;
mod control;
mod grants;
mod lookup;
mod namespace;
mod plain;
mod profile;
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

const TRANSFER_LIMIT: usize = 2;

pub struct Server7<'a> {
    volume: &'a mut Volume7,
    grants: grants::Grants,
    writes: write::Writes,
    plain: plain::PlainTransfers,
}

impl<'a> Server7<'a> {
    pub fn new(volume: &'a mut Volume7) -> Self {
        let writes = write::Writes::new(volume);
        Self {
            volume,
            grants: grants::Grants::new(),
            writes,
            plain: plain::PlainTransfers::new(),
        }
    }

    /// The served volume, for read-only inspection by the owner.
    pub fn volume(&self) -> &Volume7 {
        self.volume
    }

    /// Number of staged and ordinary replacement candidates still owned by clients.
    pub fn pending(&self) -> usize {
        self.writes.transfer_count() + self.plain.transfer_count()
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
        let result = retention::maintain(
            self.volume,
            disk,
            self.writes.transfers_open() || self.plain.transfers_open(),
        );
        if result.is_ok() || self.volume.header().is_err() {
            self.writes.forget_receipts();
        }
        result
    }

    /// Serve one client request, settling any controlled publication inside
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

    /// Serve one client request, driving tracked and admission publication over the
    /// pollable `disk` and calling `control` before each of its polls. The
    /// callback returns the owner's clock and may revoke or detach slots
    /// through its [`Control7`]; it must not block indefinitely, and it is
    /// never called for requests without a publication.
    ///
    /// If the caller loses its authority before the publication's header is
    /// submitted, nothing of it becomes durable: an admission execution is then
    /// recorded cancelled with cause `AuthorityLost` by a second, unstoppable
    /// publication, and the reply is the authority error. Later losses let
    /// the publication settle: a new admission is then likewise cancelled
    /// with `AuthorityLost` before the authority error is returned, while a
    /// settled tracked commit, execution or cancellation stands and is reported `Uncertain`.
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
            LOOKUP | STAT | LIST => {
                crate::validation::request(&packet)?;
                namespace::request(self.volume, grant, packet)
            }
            READ | REFERENCES | READ_OPEN | READ_CHUNK => {
                crate::validation::request(&packet)?;
                read::request(self.volume, disk, grant, packet)
            }
            _ if plain::selected(&packet) => {
                crate::validation::request(&packet)?;
                let profile_busy = self.writes.transfer_open(slot)
                    || self.plain.transfer_count() + self.writes.transfer_count() >= TRANSFER_LIMIT;
                self.plain
                    .request(self.volume, disk, slot, grant, packet, profile_busy)
            }
            REPLACE_COMMIT => self.with_owner(disk, slot, grant, packet, control),
            _ if write::selected(&packet) => {
                if packet.op == REPLACE_OPEN {
                    self.ensure_profile_open_available(slot, grant, &packet)?;
                }
                self.writes.request(self.volume, disk, slot, grant, packet)
            }
            _ if admission::selected(&packet) => {
                if packet.op == rustic_abi::files::admission::OPEN {
                    self.ensure_profile_open_available(slot, grant, &packet)?;
                }
                self.with_owner(disk, slot, grant, packet, control)
            }
            _ => Err(Error::Unsupported),
        }
    }

    /// Coordinate the shared authority lifetime around tracked or admission
    /// publication. Each operation's own module decides its durable outcome.
    fn with_owner<D: Disk + PollDisk7>(
        &mut self,
        disk: &mut D,
        slot: usize,
        grant: Grant7,
        packet: Packet,
        control: &mut dyn FnMut(&mut Control7<'_>) -> u64,
    ) -> Result<Packet, Error> {
        let caller = control::Caller7 {
            slot,
            peer: grant.peer,
            context: packet.context,
        };
        let mut transfers = 0;
        for index in 0..CLIENTS7 {
            if self.writes.transfer_open(index) || self.plain.transfer_open(index) {
                transfers |= 1 << index;
            }
        }
        let mut owner = control::Owner::new(&mut self.grants, caller, transfers, control);
        let result = if packet.op == REPLACE_COMMIT {
            self.writes
                .commit_with(self.volume, disk, slot, grant, packet, &mut owner)
        } else {
            admission::request(
                &mut self.writes,
                self.volume,
                disk,
                slot,
                grant,
                packet,
                &mut owner,
            )
        };
        // The publication has released the volume; drop lost candidates now.
        let lost = owner.lost();
        self.reset_lost(lost);
        result
    }

    /// Check the plain/tracked/admission shared transfer limit after decoding and
    /// validating the open request's authority, before its owner allocates a
    /// V7 stage.
    fn ensure_profile_open_available(
        &self,
        slot: usize,
        grant: Grant7,
        packet: &Packet,
    ) -> Result<(), Error> {
        let request = profile::decode_open(packet)?.request;
        grant.holds(WRITE_RIGHT | INSPECT_RIGHT)?;
        if grant.subject == 0 || slot >= CLIENTS7 {
            return Err(Error::Denied);
        }
        scope::authorized_resource(
            self.volume,
            grant,
            request.workspace.root(),
            request.resource.object(),
        )?;
        let header = self.volume.header().map_err(crate::reply::error)?;
        if header.lineage != request.workspace.lineage() {
            return Err(Error::Denied);
        }
        if self.plain.transfer_open(slot)
            || self.plain.transfer_count() + self.writes.transfer_count() >= TRANSFER_LIMIT
        {
            return Err(Error::Busy);
        }
        Ok(())
    }
}
