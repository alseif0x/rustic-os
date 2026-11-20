// SPDX-License-Identifier: Apache-2.0
//! Profile-1, profile-2 and legacy tracked replacement: open, streamed chunks,
//! commit with the selected existing receipt framing, abort, retained lookups
//! and receipt parts for profile-1/profile-2 operations.
//!
//! The per-slot transfer table is shared with staged admission: one slot holds
//! at most one transfer of either stage kind, and each kind's requests can only
//! continue, finish or abort a transfer of that kind.
//!
//! Policy lives here: which rights each step needs, which retry subject and
//! service instance are persisted, and when transfer state is dropped. The
//! volume enforces versions, retry scopes, the retained-record budget and
//! publication barriers.
mod legacy;
mod publication;

use super::lookup::{self, receipt};
use super::profile;
use super::transfer::{Fault, Transfer};
use super::{CLIENTS7, Grant7, scope};
use crate::reply;
use rustic_abi::files::{operation, workspace::Operation, *};
use rustic_fs::{
    Disk, Stage7Kind, Volume7, WriteIdentity7,
    format7::{Record7, RecordState},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Framing {
    Modern,
    Legacy,
}

#[derive(Clone, Copy)]
struct TransferBinding {
    kind: Stage7Kind,
    framing: Framing,
}

impl TransferBinding {
    fn modern(kind: Stage7Kind) -> Self {
        Self {
            kind,
            framing: Framing::Modern,
        }
    }

    fn legacy() -> Self {
        Self {
            kind: Stage7Kind::Tracked,
            framing: Framing::Legacy,
        }
    }
}

/// Whether `p` selects the tracked-write path. OPEN and lookup decoders enforce
/// the exact profile-1 or profile-2 shape; chunk, commit and abort bind to the
/// profile and stage kind recorded by the open transfer.
pub(super) fn selected(p: &Packet) -> bool {
    matches!(
        p.op,
        REPLACE_OPEN
            | OPERATION_ID
            | OPERATION_PART
            | OPERATION_RETRY
            | REPLACE_CHUNK
            | REPLACE_COMMIT
            | REPLACE_ABORT
    )
}

/// Per-slot transfers and the last receipt each slot produced or looked up.
pub(super) struct Writes {
    transfers: [Option<Transfer>; CLIENTS7],
    receipts: [Option<Operation>; CLIENTS7],
    /// Service instance persisted in fresh records: the first sequence this
    /// mount can commit. Zero when the volume was not ready at construction.
    instance: u64,
}

impl Writes {
    pub(super) fn new(volume: &Volume7) -> Self {
        let instance = volume
            .header()
            .ok()
            .and_then(|header| header.sequence.checked_add(1))
            .unwrap_or(0);
        Self {
            transfers: [const { None }; CLIENTS7],
            receipts: [None; CLIENTS7],
            instance,
        }
    }

    /// Drop everything `slot` holds: abort its stage and forget its receipt.
    pub(super) fn reset(&mut self, volume: &mut Volume7, slot: usize) {
        if let Some(transfer) = self.transfers.get_mut(slot).and_then(Option::take) {
            transfer.abort(volume);
        }
        if let Some(receipt) = self.receipts.get_mut(slot) {
            *receipt = None;
        }
    }

    /// Whether any slot has a transfer open.
    pub(super) fn transfers_open(&self) -> bool {
        self.transfers.iter().any(Option::is_some)
    }

    /// Number of streamed transfer profiles holding a V7 storage stage.
    pub(super) fn transfer_count(&self) -> usize {
        self.transfers.iter().flatten().count()
    }

    /// Whether this client slot already owns a tracked or admission transfer.
    pub(super) fn transfer_open(&self, slot: usize) -> bool {
        self.transfers.get(slot).is_some_and(Option::is_some)
    }

    /// Forget every slot's cached receipt, after the records they describe
    /// were reclaimed.
    pub(super) fn forget_receipts(&mut self) {
        self.receipts = [None; CLIENTS7];
    }

    pub(super) fn request(
        &mut self,
        volume: &mut Volume7,
        disk: &mut impl Disk,
        slot: usize,
        grant: Grant7,
        p: Packet,
    ) -> Result<Packet, Error> {
        if slot >= CLIENTS7 {
            return Err(Error::Denied);
        }
        match p.op {
            OPERATION_PART => self.part(volume, slot, grant, p),
            OPERATION_ID | OPERATION_RETRY => {
                if grant.subject == 0 {
                    return Err(Error::Denied);
                }
                grant.holds(INSPECT_RIGHT)?;
                let (profile, query) = profile::decode_lookup(&p)?;
                let receipt = lookup::retained(volume, disk, grant, profile, query)?;
                let first = profile::receipt_part(profile, receipt, p.op, p.context, 0)?;
                self.receipts[slot] = Some(receipt);
                Ok(first)
            }
            REPLACE_OPEN => {
                let opening = profile::decode_open(&p)?;
                self.open(volume, slot, grant, opening, Stage7Kind::Tracked)?;
                Ok(ack(&p))
            }
            REPLACE_CHUNK => self.chunk(volume, disk, slot, grant, Stage7Kind::Tracked, p),
            REPLACE_ABORT => self.abort(volume, slot, grant, Stage7Kind::Tracked, p),
            _ => Err(Error::Unsupported),
        }
    }

    /// Open a streamed transfer of `size` bytes whose stage finishes as `kind`.
    ///
    /// Both kinds need write and inspection (the finish reply is a receipt or
    /// an admission status) and a nonzero retry subject. An admission may not
    /// reuse a retry identity that names a direct tracked write.
    pub(super) fn open(
        &mut self,
        volume: &mut Volume7,
        slot: usize,
        grant: Grant7,
        opening: profile::Opening,
        kind: Stage7Kind,
    ) -> Result<(), Error> {
        self.open_with_policy(volume, slot, grant, opening, kind, false)
    }

    fn open_with_policy(
        &mut self,
        volume: &mut Volume7,
        slot: usize,
        grant: Grant7,
        opening: profile::Opening,
        kind: Stage7Kind,
        legacy_replay: bool,
    ) -> Result<(), Error> {
        let profile = opening.profile;
        let request = opening.request;
        let size = opening.size;
        grant.holds(if legacy_replay {
            INSPECT_RIGHT
        } else {
            WRITE_RIGHT | INSPECT_RIGHT
        })?;
        if grant.subject == 0 || slot >= CLIENTS7 {
            return Err(Error::Denied);
        }
        let workspace = request.workspace.root();
        let object = request.resource.object();
        if legacy_replay {
            if !scope::retained_visible(volume, grant, workspace, object) {
                return Err(Error::OutcomeUnknown);
            }
        } else {
            scope::authorized_resource(volume, grant, workspace, object)?;
        }
        let header = volume.header().map_err(reply::error)?;
        if header.lineage != request.workspace.lineage() {
            return Err(Error::Denied);
        }
        if self.transfers[slot].is_some() {
            return Err(Error::Busy);
        }
        if self.instance == 0 && !legacy_replay {
            return Err(Error::Uncertain);
        }
        let epoch = request.retry.epoch.value();
        let key = request.retry.key.value();
        // An exact retry must present the instance its record persisted, which
        // may belong to an earlier mount; a fresh operation uses this mount's.
        let retained = volume
            .retained_records()
            .map_err(reply::error)?
            .iter()
            .flatten()
            .find(|record| {
                record.subject == grant.subject
                    && record.workspace == workspace
                    && record.retry_epoch == epoch
                    && record.retry_key == key
            })
            .copied();
        if kind == Stage7Kind::Admission
            && retained.is_some_and(|record| record.state == RecordState::DirectCommitted)
        {
            return Err(Error::IdempotencyConflict);
        }
        let identity = WriteIdentity7 {
            subject: grant.subject,
            workspace,
            object,
            instance: retained.map_or(self.instance, |record| record.instance),
            retry_epoch: epoch,
            retry_key: key,
        };
        let stage = volume
            .open_stage(identity, request.expected_version.value(), size, kind)
            .map_err(reply::error)?;
        self.transfers[slot] = Some(Transfer::new(
            stage,
            kind,
            profile,
            legacy_replay,
            request,
            size,
        ));
        Ok(())
    }

    /// Accept the next client bytes of this slot's `kind` transfer.
    pub(super) fn chunk(
        &mut self,
        volume: &mut Volume7,
        disk: &mut impl Disk,
        slot: usize,
        grant: Grant7,
        kind: Stage7Kind,
        p: Packet,
    ) -> Result<Packet, Error> {
        grant.holds(WRITE_RIGHT)?;
        self.chunk_profile(volume, disk, slot, grant, p, TransferBinding::modern(kind))
    }

    fn chunk_profile(
        &mut self,
        volume: &mut Volume7,
        disk: &mut impl Disk,
        slot: usize,
        grant: Grant7,
        p: Packet,
        binding: TransferBinding,
    ) -> Result<Packet, Error> {
        if p.count == 0 || p.version != 0 {
            return Err(Error::Protocol);
        }
        let transfer = self.transfer(slot, p.id, binding)?;
        let rights = transfer_rights(transfer, binding.framing);
        grant.holds(rights)?;
        match transfer.chunk(volume, disk, p.arg, p.payload()) {
            Ok(()) => Ok(ack(&p)),
            Err(Fault::Refused(error)) => Err(error),
            Err(Fault::Ended(error)) => {
                self.transfers[slot] = None;
                Err(error)
            }
        }
    }

    /// Remove this slot's complete `kind` transfer so it can be finished. An
    /// incomplete transfer stays open and is refused with `Offset`.
    pub(super) fn take_complete(
        &mut self,
        slot: usize,
        grant: Grant7,
        kind: Stage7Kind,
        p: &Packet,
    ) -> Result<Transfer, Error> {
        grant.holds(WRITE_RIGHT)?;
        self.take_complete_profile(slot, grant, p, TransferBinding::modern(kind))
    }

    fn take_complete_profile(
        &mut self,
        slot: usize,
        grant: Grant7,
        p: &Packet,
        binding: TransferBinding,
    ) -> Result<Transfer, Error> {
        bare(p)?;
        let transfer = self.transfer(slot, p.id, binding)?;
        let rights = transfer_rights(transfer, binding.framing);
        grant.holds(rights)?;
        if !transfer.complete() {
            return Err(Error::Offset);
        }
        self.transfers[slot].take().ok_or(Error::NoTransfer)
    }

    /// Release this slot's `kind` transfer without I/O.
    pub(super) fn abort(
        &mut self,
        volume: &mut Volume7,
        slot: usize,
        grant: Grant7,
        kind: Stage7Kind,
        p: Packet,
    ) -> Result<Packet, Error> {
        grant.holds(WRITE_RIGHT)?;
        self.abort_profile(volume, slot, grant, p, TransferBinding::modern(kind))
    }

    fn abort_profile(
        &mut self,
        volume: &mut Volume7,
        slot: usize,
        grant: Grant7,
        p: Packet,
        binding: TransferBinding,
    ) -> Result<Packet, Error> {
        bare(&p)?;
        self.transfer(slot, p.id, binding)?;
        grant.holds(if binding.framing == Framing::Legacy {
            INSPECT_RIGHT
        } else {
            WRITE_RIGHT
        })?;
        if let Some(transfer) = self.transfers[slot].take() {
            transfer.abort(volume);
        }
        Ok(ack(&p))
    }

    /// Receipt parts are served only for the operation this slot last
    /// completed or looked up; a lookup by ID or retry identity recomputes a
    /// retained record's receipt first.
    fn part(
        &self,
        volume: &Volume7,
        slot: usize,
        grant: Grant7,
        p: Packet,
    ) -> Result<Packet, Error> {
        grant.holds(INSPECT_RIGHT)?;
        let (profile, query) = profile::decode_lookup(&p)?;
        let operation::Lookup::Id(id) = query else {
            return Err(Error::Protocol);
        };
        let receipt = self.receipts[slot]
            .filter(|receipt| receipt.id == id)
            .ok_or(Error::OutcomeUnknown)?;
        if !scope::retained_visible(
            volume,
            grant,
            receipt.workspace.root(),
            receipt.resource.object(),
        ) {
            return Err(Error::OutcomeUnknown);
        }
        profile::receipt_part(profile, receipt, OPERATION_PART, p.context, p.arg as usize)
    }

    /// This slot's open transfer, when it targets `object` and finishes as
    /// `kind`; a transfer of the other kind is not addressable here.
    fn transfer(
        &mut self,
        slot: usize,
        object: u32,
        binding: TransferBinding,
    ) -> Result<&mut Transfer, Error> {
        let transfer = self
            .transfers
            .get_mut(slot)
            .and_then(Option::as_mut)
            .filter(|transfer| transfer.object() == object && transfer.kind() == binding.kind)
            .ok_or(Error::NoTransfer)?;
        if match binding.framing {
            Framing::Modern => transfer.profile() == profile::Profile::Legacy,
            Framing::Legacy => transfer.profile() != profile::Profile::Legacy,
        } {
            return Err(Error::NoTransfer);
        }
        Ok(transfer)
    }
}

/// An empty acknowledgement of `p`.
fn ack(p: &Packet) -> Packet {
    let mut ack = Packet::new(p.op);
    ack.context = p.context;
    ack
}

fn transfer_rights(transfer: &Transfer, framing: Framing) -> u8 {
    if framing == Framing::Modern {
        return WRITE_RIGHT;
    }
    if transfer.legacy_replay() {
        INSPECT_RIGHT
    } else {
        WRITE_RIGHT | INSPECT_RIGHT
    }
}

/// Commit, accept and abort carry only the object identity.
fn bare(p: &Packet) -> Result<(), Error> {
    if p.count != 0 || p.arg != 0 || p.version != 0 {
        return Err(Error::Protocol);
    }
    Ok(())
}

/// The receipt for a record `finish_tracked` has already published, checked
/// against the request it answers. Any failure is [`Error::Uncertain`]: the
/// effect exists even though the service cannot describe it; an exact retry or
/// a later lookup can still recover it.
fn committed(
    lineage: [u8; 16],
    record: &Record7,
    sha256: [u8; 32],
    request: operation::Replacement,
) -> Result<Operation, Error> {
    let receipt = receipt(lineage, record, sha256).map_err(|_| Error::Uncertain)?;
    if receipt.workspace != request.workspace
        || receipt.resource != request.resource
        || receipt.retry != request.retry
        || receipt.previous_version != request.expected_version
    {
        return Err(Error::Uncertain);
    }
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustic_abi::files::{
        operation::{Key, Retry},
        reference::{Epoch, References, Version},
    };

    const LINEAGE: [u8; 16] = [0x5c; 16];

    fn request() -> operation::Replacement {
        let references = References::new(LINEAGE, 5, 8).unwrap();
        operation::Replacement {
            workspace: references.workspace,
            resource: references.resource,
            expected_version: Version::new(7).unwrap(),
            retry: Retry {
                epoch: Epoch::new(1).unwrap(),
                key: Key::new(9).unwrap(),
            },
        }
    }

    fn record() -> Record7 {
        Record7 {
            subject: 2,
            workspace: 5,
            object: 8,
            instance: 8,
            retry_epoch: 1,
            retry_key: 9,
            previous: 7,
            committed: 8,
            admission_number: 0,
            terminal: 8,
            length: 3,
            payload_crc32: 0,
            state: RecordState::DirectCommitted,
            prevention: None,
            extents_used: 0,
            extents: [rustic_fs::Extent::new(0, 0); rustic_fs::EXTENTS_PER_FILE],
        }
    }

    #[test]
    fn a_consistent_record_becomes_the_receipt() {
        let receipt = committed(LINEAGE, &record(), [1; 32], request()).unwrap();
        assert_eq!(receipt.version.value(), 8);
        assert_eq!(receipt.size, 3);
    }

    #[test]
    fn an_indescribable_or_inconsistent_committed_record_is_uncertain() {
        let mut other_object = record();
        other_object.object = 9;
        let mut other_previous = record();
        other_previous.previous = 6;
        let mut unrepresentable = record();
        unrepresentable.instance = 0;
        for bad in [other_object, other_previous, unrepresentable] {
            assert_eq!(
                committed(LINEAGE, &bad, [1; 32], request()),
                Err(Error::Uncertain)
            );
        }
        assert_eq!(
            committed([0; 16], &record(), [1; 32], request()),
            Err(Error::Uncertain)
        );
    }
}
