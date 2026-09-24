// SPDX-License-Identifier: Apache-2.0
//! Profile-2 lookups of retained records by operation ID or retry identity,
//! answered with a completed-operation receipt whose SHA-256 is recomputed by
//! streaming the retained snapshot one sector at a time.
//!
//! Policy mirrors the v5 lookups: the caller needs `INSPECT` and a nonzero
//! subject, only records of that subject exist for it, and a record outside
//! the grant scope is indistinguishable from a missing one. The digest relies
//! on the mount-time CRC verification of every retained snapshot; it is not a
//! fresh integrity check of the medium.
use super::{Grant7, scope};
use crate::reply;
use rustic_abi::files::{
    operation::{self, Instance, Key, OperationId, Retry},
    reference::{Epoch, Resource, Version, Workspace},
    workspace::{Lookup, Operation},
    *,
};
use rustic_fs::{
    Disk, Volume7,
    format7::{Record7, RecordState},
};
use sha2::{Digest, Sha256};

/// Sector-sized buffer used to stream a retained snapshot into the digest.
const SECTOR: usize = 512;

/// Resolve an `OPERATION_ID` or `OPERATION_RETRY` lookup to its receipt.
pub(super) fn retained(
    volume: &Volume7,
    disk: &mut impl Disk,
    grant: Grant7,
    p: &Packet,
) -> Result<Operation, Error> {
    if grant.subject == 0 {
        return Err(Error::Denied);
    }
    grant.holds(INSPECT_RIGHT)?;
    let query = Lookup::decode(p)?.query;
    let header = volume.header().map_err(reply::error)?;
    let records = volume.retained_records().map_err(reply::error)?;
    let mine = || {
        records
            .iter()
            .flatten()
            .filter(|r| r.subject == grant.subject)
    };
    let record = match query {
        operation::Lookup::Id(id) => {
            if id.lineage() != header.lineage {
                return Err(Error::Lineage);
            }
            mine()
                .find(|record| record.committed == id.sequence())
                .ok_or(Error::OutcomeUnknown)?
        }
        operation::Lookup::Retry { workspace, retry } => {
            if workspace.lineage() != header.lineage {
                return Err(Error::Lineage);
            }
            let epoch = retry.epoch.value();
            // Missing and out-of-scope give the same answer, which for a key
            // outside the current epoch is `ExpiredEpoch`, as in v5.
            let hidden = if epoch == header.epoch {
                Error::OutcomeUnknown
            } else {
                Error::ExpiredEpoch
            };
            mine()
                .find(|record| {
                    record.workspace == workspace.root()
                        && record.retry_epoch == epoch
                        && record.retry_key == retry.key.value()
                })
                .filter(|record| visible(volume, grant, record))
                .ok_or(hidden)?
        }
    };
    // A guessed identity must not reveal history outside this grant's scope.
    if !visible(volume, grant, record) {
        return Err(Error::OutcomeUnknown);
    }
    match record.state {
        RecordState::DirectCommitted | RecordState::AdmittedCommitted => {}
        RecordState::Admitted => return Err(Error::Busy),
        // The completed-only receipt profile cannot describe a cancellation.
        RecordState::Cancelled => return Err(Error::Unsupported),
    }
    let sha256 = snapshot_sha256(volume, disk, record)?;
    receipt(header.lineage, record, sha256)
}

/// Whether the record lies within the grant scope in the live namespace.
fn visible(volume: &Volume7, grant: Grant7, record: &Record7) -> bool {
    scope::retained_visible(volume, grant.scope, record.workspace, record.object)
}

/// SHA-256 of a retained snapshot, streamed through one sector buffer.
fn snapshot_sha256(
    volume: &Volume7,
    disk: &mut impl Disk,
    record: &Record7,
) -> Result<[u8; 32], Error> {
    let mut digest = Sha256::new();
    let mut block = [0u8; SECTOR];
    let mut offset = 0u64;
    while offset < u64::from(record.length) {
        let read = volume
            .read_retained_range(disk, record, offset, &mut block)
            .map_err(reply::error)?;
        if read == 0 {
            return Err(Error::Corrupt);
        }
        digest.update(&block[..read]);
        offset += read as u64;
    }
    Ok(digest.finalize().into())
}

/// The completed-operation receipt for a committed retained record.
pub(super) fn receipt(
    lineage: [u8; 16],
    record: &Record7,
    sha256: [u8; 32],
) -> Result<Operation, Error> {
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
