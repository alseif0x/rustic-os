// SPDX-License-Identifier: Apache-2.0
//! Profile-2 tracked replacement: open, streamed chunks, commit with a
//! completed-operation receipt, abort, and receipt parts for this slot.
//!
//! Policy lives here: which rights each step needs, which retry subject and
//! service instance are persisted, and when transfer state is dropped. The
//! volume enforces versions, retry scopes, the retained-record budget and
//! publication barriers.
use super::transfer::{Fault, Transfer};
use super::{CLIENTS7, Grant7, scope};
use crate::reply;
use rustic_abi::files::{
    operation::{self, Instance, Key, OperationId, Retry},
    reference::{Epoch, Resource, Version, Workspace},
    workspace::{Lookup, Operation, Replacement},
    *,
};
use rustic_fs::{Disk, Stage7Kind, Volume7, WriteIdentity7, format7::Record7};

/// Whether `p` selects the profile-2 write path. Open and lookups carry an
/// explicit profile marker; chunk, commit and abort bind to an open transfer.
pub(super) fn selected(p: &Packet) -> bool {
    match p.op {
        REPLACE_OPEN => p.count == 40,
        OPERATION_PART => p.count == 20,
        REPLACE_CHUNK | REPLACE_COMMIT | REPLACE_ABORT => true,
        _ => false,
    }
}

/// Per-slot transfers and the last receipt each slot produced.
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
        let mut ack = Packet::new(p.op);
        ack.context = p.context;
        match p.op {
            OPERATION_PART => self.part(slot, grant, p),
            REPLACE_OPEN => {
                self.open(volume, slot, grant, p)?;
                Ok(ack)
            }
            REPLACE_CHUNK => {
                grant.holds(WRITE_RIGHT)?;
                if p.count == 0 || p.version != 0 {
                    return Err(Error::Protocol);
                }
                let transfer = self.transfer(slot, p.id)?;
                match transfer.chunk(volume, disk, p.arg, p.payload()) {
                    Ok(()) => Ok(ack),
                    Err(Fault::Refused(error)) => Err(error),
                    Err(Fault::Ended(error)) => {
                        self.transfers[slot] = None;
                        Err(error)
                    }
                }
            }
            REPLACE_COMMIT => {
                grant.holds(WRITE_RIGHT)?;
                bare(&p)?;
                if !self.transfer(slot, p.id)?.complete() {
                    return Err(Error::Offset);
                }
                let transfer = self.transfers[slot].take().ok_or(Error::NoTransfer)?;
                let request = transfer.request();
                let (record, sha256) = transfer.finish(volume, disk)?;
                // The effect is committed from here on: any failure to describe
                // it is `Uncertain`, never an error that implies no effect.
                let receipt = volume
                    .header()
                    .map_err(|_| Error::Uncertain)
                    .and_then(|header| committed(header.lineage, &record, sha256, request))?;
                let first = receipt
                    .part(REPLACE_COMMIT, p.context, 0)
                    .map_err(|_| Error::Uncertain)?;
                self.receipts[slot] = Some(receipt);
                Ok(first)
            }
            REPLACE_ABORT => {
                grant.holds(WRITE_RIGHT)?;
                bare(&p)?;
                self.transfer(slot, p.id)?;
                if let Some(transfer) = self.transfers[slot].take() {
                    transfer.abort(volume);
                }
                Ok(ack)
            }
            _ => Err(Error::Unsupported),
        }
    }

    fn open(
        &mut self,
        volume: &mut Volume7,
        slot: usize,
        grant: Grant7,
        p: Packet,
    ) -> Result<(), Error> {
        // The commit reply is a receipt, so writing also needs inspection.
        grant.holds(WRITE_RIGHT | INSPECT_RIGHT)?;
        if grant.subject == 0 {
            return Err(Error::Denied);
        }
        let request = Replacement::decode(&p)?.request;
        let workspace = request.workspace.root();
        let object = request.resource.object();
        scope::authorized_resource(volume, grant.scope, workspace, object)?;
        let header = volume.header().map_err(reply::error)?;
        if header.lineage != request.workspace.lineage() {
            return Err(Error::Denied);
        }
        if self.transfers[slot].is_some() {
            return Err(Error::Busy);
        }
        if self.instance == 0 {
            return Err(Error::Uncertain);
        }
        let epoch = request.retry.epoch.value();
        let key = request.retry.key.value();
        // An exact retry must present the instance its record persisted, which
        // may belong to an earlier mount; a fresh operation uses this mount's.
        let instance = volume
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
            .map_or(self.instance, |record| record.instance);
        let identity = WriteIdentity7 {
            subject: grant.subject,
            workspace,
            object,
            instance,
            retry_epoch: epoch,
            retry_key: key,
        };
        let stage = volume
            .open_stage(
                identity,
                request.expected_version.value(),
                p.arg,
                Stage7Kind::Tracked,
            )
            .map_err(reply::error)?;
        self.transfers[slot] = Some(Transfer::new(stage, request, p.arg));
        Ok(())
    }

    /// Receipt parts are served only for the operation this slot last
    /// completed; cold lookups of retained records are not implemented.
    fn part(&self, slot: usize, grant: Grant7, p: Packet) -> Result<Packet, Error> {
        grant.holds(INSPECT_RIGHT)?;
        let operation::Lookup::Id(id) = Lookup::decode(&p)?.query else {
            return Err(Error::Protocol);
        };
        let receipt = self.receipts[slot]
            .filter(|receipt| receipt.id == id)
            .ok_or(Error::OutcomeUnknown)?;
        receipt.part(OPERATION_PART, p.context, p.arg as usize)
    }

    fn transfer(&mut self, slot: usize, object: u32) -> Result<&mut Transfer, Error> {
        self.transfers[slot]
            .as_mut()
            .filter(|transfer| transfer.object() == object)
            .ok_or(Error::NoTransfer)
    }
}

/// Commit and abort carry only the object identity.
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

/// The completed-operation receipt for a direct-committed record.
fn receipt(lineage: [u8; 16], record: &Record7, sha256: [u8; 32]) -> Result<Operation, Error> {
    let workspace = Workspace::new(lineage, record.workspace)?;
    Ok(Operation {
        id: OperationId::new(lineage, record.committed)?,
        service_instance: Instance::new(lineage, record.instance)?,
        workspace,
        resource: Resource::new(workspace, record.object)?,
        previous_version: Version::new(record.previous)?,
        version: Version::new(record.committed)?,
        size: record.length,
        retry: Retry {
            epoch: Epoch::new(record.retry_epoch)?,
            key: Key::new(record.retry_key)?,
        },
        sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustic_abi::files::reference::References;
    use rustic_fs::format7::RecordState;

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
