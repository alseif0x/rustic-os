// SPDX-License-Identifier: Apache-2.0
//! Legacy flat tracked-write framing over the shared V7 tracked stage.

use super::{TransferBinding, Writes};
use crate::v7::{
    Grant7,
    control::{Owner, drive},
    profile,
};
use rustic_abi::files::{Error, INSPECT_RIGHT, Packet, WRITE_RIGHT};
use rustic_fs::{Disk, PollDisk7, Stage7Kind, Volume7};

impl Writes {
    /// Open the already-resolved flat retry identity. Inspection-only authority
    /// is sufficient only for a unique retained direct record.
    pub(in super::super) fn open_legacy(
        &mut self,
        volume: &mut Volume7,
        slot: usize,
        grant: Grant7,
        opening: profile::Opening,
        replay: bool,
    ) -> Result<(), Error> {
        if opening.profile != profile::Profile::Legacy {
            return Err(Error::Protocol);
        }
        self.open_with_policy(volume, slot, grant, opening, Stage7Kind::Tracked, replay)
    }

    /// Accept one generic CHUNK only for a TRACK_BEGIN candidate. Profile-1,
    /// profile-2 and admission stages remain open if the caller crosses modes.
    pub(in super::super) fn chunk_legacy(
        &mut self,
        volume: &mut Volume7,
        disk: &mut impl Disk,
        slot: usize,
        grant: Grant7,
        packet: Packet,
    ) -> Result<Packet, Error> {
        self.chunk_profile(volume, disk, slot, grant, packet, TransferBinding::legacy())
    }

    /// Remove a complete generic COMMIT candidate without consuming a
    /// differently framed transfer.
    pub(in super::super) fn take_complete_legacy(
        &mut self,
        slot: usize,
        grant: Grant7,
        packet: &Packet,
    ) -> Result<super::Transfer, Error> {
        self.take_complete_profile(slot, grant, packet, TransferBinding::legacy())
    }

    /// Release a generic ABORT only when TRACK_BEGIN opened the candidate.
    pub(in super::super) fn abort_legacy(
        &mut self,
        volume: &mut Volume7,
        slot: usize,
        grant: Grant7,
        packet: Packet,
    ) -> Result<Packet, Error> {
        self.abort_profile(volume, slot, grant, packet, TransferBinding::legacy())
    }

    /// Settle a generic legacy COMMIT under V7 owner control. The flat key is
    /// rescanned before candidate construction so intervening scoped commits
    /// cannot make this stage publish from a stale namespace view.
    pub(in super::super) fn commit_legacy_with<D: PollDisk7>(
        &mut self,
        volume: &mut Volume7,
        disk: &mut D,
        slot: usize,
        grant: Grant7,
        packet: Packet,
        owner: &mut Owner<'_>,
    ) -> Result<Packet, Error> {
        let transfer = self.take_complete_legacy(slot, grant, &packet)?;
        let request = transfer.request();
        let length = transfer.size();
        let replay = transfer.legacy_replay();
        owner.consumed(slot);

        if let Err(error) =
            super::super::recovery::recheck_commit(volume, grant, request, length, replay)
        {
            transfer.abort(volume);
            return Err(error);
        }

        let required = if replay {
            INSPECT_RIGHT
        } else {
            WRITE_RIGHT | INSPECT_RIGHT
        };
        let result = transfer.finish_with(volume, disk, |publication| {
            drive(owner, publication, Some(required))
        });
        if volume.header().is_err() {
            self.forget_receipts();
        }
        let (driven, _) = result?;
        if let Some(error) = driven.denied {
            return Err(if driven.record.is_some() {
                Error::Uncertain
            } else {
                error
            });
        }
        let record = driven.record.ok_or(Error::Uncertain)?;
        let lineage = volume.header().map_err(|_| Error::Uncertain)?.lineage;
        let receipt = super::super::recovery::legacy_receipt(lineage, record)
            .map_err(|_| Error::Uncertain)?;
        let mut response = Packet::new(packet.op);
        response.context = packet.context;
        Ok(receipt.packet(response))
    }
}
