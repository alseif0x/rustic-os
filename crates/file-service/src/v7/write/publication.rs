// SPDX-License-Identifier: Apache-2.0
//! Tracked commit settlement with the same owner control as V7 admission.
use super::{Writes, committed};
use crate::v7::{
    Grant7,
    control::{Owner, drive},
    profile,
};
use rustic_abi::files::{Error, INSPECT_RIGHT, Packet, REPLACE_COMMIT, WRITE_RIGHT};
use rustic_fs::{PollDisk7, Stage7Kind, Volume7};

impl Writes {
    /// A direct tracked commit has no durable admitted state to cancel. Before
    /// its header, authority loss discards the candidate; after submission the
    /// effect settles and the receipt is withheld until a fresh authorized query.
    pub(in super::super) fn commit_with<D: PollDisk7>(
        &mut self,
        volume: &mut Volume7,
        disk: &mut D,
        slot: usize,
        grant: Grant7,
        packet: Packet,
        owner: &mut Owner<'_>,
    ) -> Result<Packet, Error> {
        let transfer = self.take_complete(slot, grant, Stage7Kind::Tracked, &packet)?;
        let wire_profile = transfer.profile();
        let request = transfer.request();
        owner.consumed(slot);
        let result = transfer.finish_with(volume, disk, |publication| {
            drive(owner, publication, Some(WRITE_RIGHT | INSPECT_RIGHT))
        });
        if volume.header().is_err() {
            self.forget_receipts();
        }
        let (driven, sha256) = result?;
        if let Some(error) = driven.denied {
            return Err(if driven.record.is_some() {
                Error::Uncertain
            } else {
                error
            });
        }
        let record = driven.record.ok_or(Error::Uncertain)?;
        let receipt = volume
            .header()
            .map_err(|_| Error::Uncertain)
            .and_then(|header| committed(header.lineage, &record, sha256, request))?;
        let first = profile::receipt_part(wire_profile, receipt, REPLACE_COMMIT, packet.context, 0)
            .map_err(|_| Error::Uncertain)?;
        self.receipts[slot] = Some(receipt);
        Ok(first)
    }
}
