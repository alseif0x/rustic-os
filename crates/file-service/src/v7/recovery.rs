// SPDX-License-Identifier: Apache-2.0
//! The existing flat RECOVERY / TRACK_BEGIN / RECEIPT profile over V7.
//!
//! New flat writes are recorded beneath the actual canonical top-level root
//! containing the target. A retained lookup scans both slots across every
//! workspace because V7 intentionally has no durable family discriminator.
//! Only one suitable direct record can be projected into the old flat reply.

use super::{Grant7, profile, scope, write::Writes};
use crate::reply;
use rustic_abi::files::{
    self, Error, INSPECT_RIGHT, Packet, WRITE_RIGHT,
    recovery::{Receipt, Retry},
    reference::{Epoch, Resource, Version, Workspace},
};
use rustic_fs::{Disk, Kind, Volume7, format7::RecordState};

/// Route flat recovery commands and generic transfer opcodes while any V7
/// stage owns the slot. The request handler decides whether its recorded wire
/// profile matches the opcode, preserving a profile-1/profile-2/admission
/// stage when a client crosses framings.
pub(super) fn selected(packet: &Packet, stage_open: bool) -> bool {
    matches!(
        packet.op,
        files::RECOVERY | files::TRACK_BEGIN | files::RECEIPT
    ) || stage_open && matches!(packet.op, files::CHUNK | files::COMMIT | files::ABORT)
}

/// Handle non-publication parts of the old bounded tracking profile. COMMIT
/// is settled by `Writes::commit_legacy_with` under V7 owner control.
pub(super) fn request(
    volume: &mut Volume7,
    disk: &mut impl Disk,
    writes: &mut Writes,
    slot: usize,
    grant: Grant7,
    packet: Packet,
    profile_busy: bool,
) -> Result<Packet, Error> {
    match packet.op {
        files::RECOVERY => recovery_info(volume, grant, packet),
        files::TRACK_BEGIN => begin(volume, writes, slot, grant, packet, profile_busy),
        files::RECEIPT => lookup(volume, grant, packet),
        files::CHUNK if writes.transfer_open(slot) => {
            writes.chunk_legacy(volume, disk, slot, grant, packet)
        }
        files::ABORT if writes.transfer_open(slot) => {
            writes.abort_legacy(volume, slot, grant, packet)
        }
        // A generic opcode cannot continue or release a modern tracked or
        // admission candidate. The transfer helper returns NoTransfer for that
        // profile and leaves it open.
        files::CHUNK | files::ABORT => Err(Error::NoTransfer),
        _ => Err(Error::Protocol),
    }
}

fn recovery_info(volume: &Volume7, grant: Grant7, packet: Packet) -> Result<Packet, Error> {
    if packet.count != 0 || packet.arg != 0 || packet.version != 0 {
        return Err(Error::Protocol);
    }
    require_subject(grant)?;
    grant.holds(INSPECT_RIGHT)?;
    inspect_identity(volume, grant, packet.id)?;
    let header = volume.header().map_err(reply::error)?;
    let mut response = Packet::new(packet.op);
    response.context = packet.context;
    response.data[..16].copy_from_slice(&header.lineage);
    response.data[16..24].copy_from_slice(&header.epoch.to_le_bytes());
    response.count = 24;
    response.arg = rustic_fs::format7::RETAINED as u32;
    Ok(response)
}

fn begin(
    volume: &mut Volume7,
    writes: &mut Writes,
    slot: usize,
    grant: Grant7,
    packet: Packet,
    profile_busy: bool,
) -> Result<Packet, Error> {
    if packet.count != 32 {
        return Err(Error::Protocol);
    }
    let length = packet.arg;
    if length as usize > files::MAX_INLINE {
        return Err(Error::Size);
    }
    if packet.id <= 4 || packet.version == 0 {
        return Err(Error::Invalid);
    }
    require_subject(grant)?;
    grant.holds(INSPECT_RIGHT)?;

    let retry = Retry::decode(packet.payload())?;
    let header = volume.header().map_err(reply::error)?;
    if retry.lineage != header.lineage {
        return Err(Error::Lineage);
    }

    let replay = match flat_match(volume, grant.subject, retry)? {
        KeyMatch::None => None,
        KeyMatch::Ambiguous => {
            return Err(ambiguous_error(volume, grant, retry));
        }
        KeyMatch::Unique(record) => {
            // Test visibility before reporting key or target mismatch so a
            // foreign record never supplies bytes or target details.
            if !scope::retained_visible(volume, grant, record.workspace, record.object) {
                return Err(Error::OutcomeUnknown);
            }
            if record.state != RecordState::DirectCommitted
                || record.object != packet.id
                || record.length != length
                || record.previous != packet.version
            {
                return Err(Error::IdempotencyConflict);
            }
            Some(record)
        }
    };

    let operation = if let Some(record) = replay {
        replacement(
            header.lineage,
            record.workspace,
            record.object,
            record.previous,
            retry,
        )?
    } else {
        if retry.epoch != header.epoch {
            return Err(Error::ExpiredEpoch);
        }
        grant.holds(WRITE_RIGHT | INSPECT_RIGHT)?;
        scope::authorized_node(volume, grant, packet.id)?;
        let workspace = canonical_root(volume, packet.id)?;
        scope::authorized_resource(volume, grant, workspace, packet.id)?;
        replacement(header.lineage, workspace, packet.id, packet.version, retry)?
    };

    if profile_busy {
        return Err(Error::Busy);
    }

    let opening = profile::Opening {
        profile: profile::Profile::Legacy,
        request: operation,
        size: length,
    };
    let replayed = replay.is_some();
    writes.open_legacy(volume, slot, grant, opening, replayed)?;
    Ok(ack(&packet))
}

fn lookup(volume: &Volume7, grant: Grant7, packet: Packet) -> Result<Packet, Error> {
    if packet.count != 32 || packet.arg != 0 || packet.version != 0 {
        return Err(Error::Protocol);
    }
    require_subject(grant)?;
    grant.holds(INSPECT_RIGHT)?;
    let retry = Retry::decode(packet.payload())?;
    let header = volume.header().map_err(reply::error)?;
    if retry.lineage != header.lineage {
        return Err(Error::Lineage);
    }

    let record = match flat_match(volume, grant.subject, retry)? {
        KeyMatch::None => {
            inspect_identity(volume, grant, packet.id)?;
            return Err(if retry.epoch == header.epoch {
                Error::OutcomeUnknown
            } else {
                Error::ExpiredEpoch
            });
        }
        KeyMatch::Ambiguous => return Err(ambiguous_error(volume, grant, retry)),
        KeyMatch::Unique(record) => record,
    };

    if !scope::retained_visible(volume, grant, record.workspace, record.object) {
        return Err(Error::OutcomeUnknown);
    }
    if record.object != packet.id || record.state != RecordState::DirectCommitted {
        return Err(Error::OutcomeUnknown);
    }
    if record.length as usize > files::MAX_INLINE {
        return Err(Error::Size);
    }
    legacy_receipt(header.lineage, *record).map(|receipt| receipt.packet(ack_seed(packet)))
}

/// Recheck flat retry ownership, scope and epoch immediately before a legacy
/// commit constructs its publication. Modern APIs retain their workspace-local
/// lookup and receive no reciprocal flat-key restriction.
pub(super) fn recheck_commit(
    volume: &Volume7,
    grant: Grant7,
    request: rustic_abi::files::operation::Replacement,
    length: u32,
    replay: bool,
) -> Result<(), Error> {
    require_subject(grant)?;
    let header = volume.header().map_err(reply::error)?;
    if request.workspace.lineage() != header.lineage {
        return Err(Error::Lineage);
    }
    let retry = Retry {
        lineage: request.workspace.lineage(),
        epoch: request.retry.epoch.value(),
        key: request.retry.key.value(),
    };
    let workspace = request.workspace.root();
    let object = request.resource.object();

    if replay {
        grant.holds(INSPECT_RIGHT)?;
        if !scope::retained_visible(volume, grant, workspace, object) {
            return Err(Error::OutcomeUnknown);
        }
        match flat_match(volume, grant.subject, retry)? {
            KeyMatch::Unique(record)
                if record.state == RecordState::DirectCommitted
                    && record.workspace == workspace
                    && record.object == object
                    && record.previous == request.expected_version.value()
                    && record.length == length =>
            {
                Ok(())
            }
            KeyMatch::Ambiguous => Err(ambiguous_error(volume, grant, retry)),
            KeyMatch::Unique(_) => Err(Error::IdempotencyConflict),
            KeyMatch::None => Err(Error::OutcomeUnknown),
        }
    } else {
        grant.holds(WRITE_RIGHT | INSPECT_RIGHT)?;
        if retry.epoch != header.epoch {
            return Err(Error::ExpiredEpoch);
        }
        scope::authorized_node(volume, grant, object)?;
        let canonical = canonical_root(volume, object)?;
        if canonical != workspace {
            return Err(Error::IdempotencyConflict);
        }
        scope::authorized_resource(volume, grant, workspace, object)?;
        match flat_match(volume, grant.subject, retry)? {
            KeyMatch::None => Ok(()),
            KeyMatch::Unique(record) => {
                if scope::retained_visible(volume, grant, record.workspace, record.object) {
                    Err(Error::IdempotencyConflict)
                } else {
                    Err(Error::OutcomeUnknown)
                }
            }
            KeyMatch::Ambiguous => Err(ambiguous_error(volume, grant, retry)),
        }
    }
}

fn flat_match(volume: &Volume7, subject: u64, retry: Retry) -> Result<KeyMatch<'_>, Error> {
    let records = volume.retained_records().map_err(reply::error)?;
    let mut found = None;
    for record in records.iter().flatten().filter(|record| {
        record.subject == subject
            && record.retry_epoch == retry.epoch
            && record.retry_key == retry.key
    }) {
        if found.is_some() {
            return Ok(KeyMatch::Ambiguous);
        }
        found = Some(record);
    }
    Ok(found.map_or(KeyMatch::None, KeyMatch::Unique))
}

fn ambiguous_error(volume: &Volume7, grant: Grant7, retry: Retry) -> Error {
    let Ok(records) = volume.retained_records() else {
        return Error::OutcomeUnknown;
    };
    let mut visible = 0;
    for record in records.iter().flatten().filter(|record| {
        record.subject == grant.subject
            && record.retry_epoch == retry.epoch
            && record.retry_key == retry.key
    }) {
        if scope::retained_visible(volume, grant, record.workspace, record.object) {
            visible += 1;
        }
    }
    if visible >= 2 {
        Error::IdempotencyConflict
    } else {
        Error::OutcomeUnknown
    }
}

fn inspect_identity(volume: &Volume7, grant: Grant7, id: u32) -> Result<(), Error> {
    if id == 0 {
        return if grant.scope == 0 {
            Ok(())
        } else {
            Err(Error::Denied)
        };
    }
    if grant.scope == id {
        return Ok(());
    }
    scope::authorized_node(volume, grant, id).map(|_| ())
}

fn require_subject(grant: Grant7) -> Result<(), Error> {
    if grant.subject == 0 {
        Err(Error::Denied)
    } else {
        Ok(())
    }
}

/// Return the actual canonical volume root (system/data/config/workspaces)
/// above a live file target. A new flat record records this root even when the
/// file also belongs to a narrower application workspace.
fn canonical_root(volume: &Volume7, object: u32) -> Result<u32, Error> {
    let mut node = volume.stat(object).map_err(|error| match error {
        rustic_fs::Error::NotFound => Error::NotFound,
        other => reply::error(other),
    })?;
    if node.kind != Kind::File || object <= 4 {
        return Err(if node.kind == Kind::Directory {
            Error::IsDirectory
        } else {
            Error::Invalid
        });
    }
    for _ in 0..rustic_fs::format7::NODES {
        if node.parent == 0 {
            return if (1..=4).contains(&node.id) && node.kind == Kind::Directory {
                Ok(node.id)
            } else {
                Err(Error::Denied)
            };
        }
        node = volume.stat(node.parent).map_err(|error| match error {
            rustic_fs::Error::NotFound => Error::Denied,
            other => reply::error(other),
        })?;
    }
    Err(Error::Corrupt)
}

fn replacement(
    lineage: [u8; 16],
    workspace: u32,
    object: u32,
    version: u64,
    retry: Retry,
) -> Result<rustic_abi::files::operation::Replacement, Error> {
    let workspace = Workspace::new(lineage, workspace)?;
    Ok(rustic_abi::files::operation::Replacement {
        workspace,
        resource: Resource::new(workspace, object)?,
        expected_version: Version::new(version)?,
        retry: rustic_abi::files::operation::Retry {
            epoch: Epoch::new(retry.epoch)?,
            key: rustic_abi::files::operation::Key::new(retry.key)?,
        },
    })
}

pub(super) fn legacy_receipt(
    lineage: [u8; 16],
    record: rustic_fs::format7::Record7,
) -> Result<Receipt, Error> {
    if record.state != RecordState::DirectCommitted || record.length as usize > files::MAX_INLINE {
        return Err(Error::OutcomeUnknown);
    }
    Ok(Receipt {
        retry: Retry {
            lineage,
            epoch: record.retry_epoch,
            key: record.retry_key,
        },
        id: record.object,
        previous: record.previous,
        committed: record.committed,
        length: u16::try_from(record.length).map_err(|_| Error::Size)?,
    })
}

fn ack(packet: &Packet) -> Packet {
    ack_seed(*packet)
}

fn ack_seed(packet: Packet) -> Packet {
    let mut response = Packet::new(packet.op);
    response.context = packet.context;
    response
}

#[derive(Clone, Copy)]
enum KeyMatch<'a> {
    None,
    Unique(&'a rustic_fs::format7::Record7),
    Ambiguous,
}
