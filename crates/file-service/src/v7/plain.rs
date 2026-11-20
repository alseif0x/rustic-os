// SPDX-License-Identifier: Apache-2.0
//! Existing untracked namespace mutations and bounded plain replacement.
//!
//! The generic legacy transfer pool already models BEGIN/CHUNK/COMMIT for
//! bounded inline bytes. V7 owns a separate instance of that pool; its
//! profile-2 writes use Volume7 stages and are capacity-coordinated by the
//! composing server.
use super::{Grant7, namespace, scope};
use crate::{reply, transfer::Transfers};
use rustic_abi::files::{Error, Packet, WRITE_RIGHT, *};
use rustic_fs::{Disk, Kind, Volume7};

const _: () = assert!(rustic_fs::MAX_FILE >= MAX_INLINE);

pub(super) struct PlainTransfers {
    transfers: Transfers,
}

impl PlainTransfers {
    pub(super) const fn new() -> Self {
        Self {
            transfers: Transfers::new(),
        }
    }

    /// Whether any slot owns an incomplete or complete candidate.
    pub(super) fn transfers_open(&self) -> bool {
        self.transfers.count() != 0
    }

    pub(super) fn transfer_count(&self) -> usize {
        self.transfers.count()
    }

    pub(super) fn transfer_open(&self, slot: usize) -> bool {
        self.transfers.open(slot)
    }

    /// Discard a client's buffered candidate on any authority lifecycle loss.
    pub(super) fn reset(&mut self, slot: usize) {
        self.transfers.clear(slot);
    }

    pub(super) fn request(
        &mut self,
        volume: &mut Volume7,
        disk: &mut impl Disk,
        slot: usize,
        grant: Grant7,
        packet: Packet,
        profile_busy: bool,
    ) -> Result<Packet, Error> {
        grant.holds(WRITE_RIGHT)?;
        match packet.op {
            CREATE | MKDIR => {
                if packet.id == 0 {
                    return Err(Error::Denied);
                }
                scope::authorized_node(volume, grant, packet.id)?;
                let kind = if packet.op == CREATE {
                    Kind::File
                } else {
                    Kind::Directory
                };
                let node = volume
                    .create(disk, packet.id, packet.payload(), kind)
                    .map_err(reply::error)?;
                Ok(namespace::node_reply(packet, node, 0))
            }
            REMOVE => {
                scope::authorized_node(volume, grant, packet.id)?;
                volume.remove(disk, packet.id).map_err(reply::error)?;
                Ok(ack(&packet))
            }
            BEGIN => self.begin(volume, slot, grant, packet, profile_busy),
            CHUNK => self.chunk(volume, slot, grant, packet),
            COMMIT => self.commit(volume, disk, slot, grant, packet),
            ABORT => self.abort(slot, packet),
            _ => Err(Error::Unsupported),
        }
    }

    fn begin(
        &mut self,
        volume: &Volume7,
        slot: usize,
        grant: Grant7,
        packet: Packet,
        profile_busy: bool,
    ) -> Result<Packet, Error> {
        let node = scope::authorized_node(volume, grant, packet.id)?;
        if node.kind != Kind::File {
            return Err(Error::IsDirectory);
        }
        if node.version != packet.version {
            return Err(Error::Version);
        }
        let total = usize::try_from(packet.arg).map_err(|_| Error::Size)?;
        if total > MAX_INLINE {
            return Err(Error::Size);
        }
        if profile_busy || self.transfer_open(slot) {
            return Err(Error::Busy);
        }
        self.transfers.begin(slot, &packet)?;
        Ok(ack(&packet))
    }

    fn chunk(
        &mut self,
        volume: &Volume7,
        slot: usize,
        grant: Grant7,
        packet: Packet,
    ) -> Result<Packet, Error> {
        scope::authorized_node(volume, grant, packet.id)?;
        self.transfers.chunk(slot, &packet)?;
        Ok(ack(&packet))
    }

    fn commit(
        &mut self,
        volume: &mut Volume7,
        disk: &mut impl Disk,
        slot: usize,
        grant: Grant7,
        packet: Packet,
    ) -> Result<Packet, Error> {
        scope::authorized_node(volume, grant, packet.id)?;
        let transfer = self.transfers.take(slot, &packet)?;
        let node = volume
            .replace(
                disk,
                transfer.id,
                transfer.version,
                &transfer.data[..transfer.total],
            )
            .map_err(reply::error)?;
        Ok(namespace::node_reply(packet, node, 0))
    }

    fn abort(&mut self, slot: usize, packet: Packet) -> Result<Packet, Error> {
        // The composing server has checked the live peer/context/expiry and
        // WRITE right. The candidate's original authorized object binding is
        // sufficient to discard it, even when another client removed the file.
        // A regrant clears candidates before installing another context.
        self.transfers.abort_plain(slot, &packet)?;
        Ok(ack(&packet))
    }
}

pub(super) fn selected(packet: &Packet) -> bool {
    matches!(
        packet.op,
        CREATE | MKDIR | REMOVE | BEGIN | CHUNK | COMMIT | ABORT
    )
}

fn ack(packet: &Packet) -> Packet {
    let mut response = Packet::new(packet.op);
    response.context = packet.context;
    response
}
