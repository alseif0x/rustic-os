// SPDX-License-Identifier: Apache-2.0
//! Explicit out-of-place migration from v5 metadata and recovery evidence.

use crate::admission::{AdmissionState, PreventionReason};
use crate::checksum::{crc, crc_update};
use crate::extent::Extent;
use crate::format::Metadata;
use crate::format7::{
    Header7, MAP_SECTORS, MAX_EXTENTS, NODE_BYTES, NODES, NODES_SECTORS, RECEIPT_BLOCK_BYTES,
    RECEIPTS_SECTORS, RECORD_BYTES, Record7, RecordState, header_sector, map_sector, nodes_sector,
    receipts_sector, validate_generation,
};
use crate::{Disk, Error, Kind, Node, storage};

use super::Volume7;
use super::payload::{PayloadPlan, payload_crc, plan_payload, reserve_plan, write_payload};

/// Copy a v5 volume into distinct disposable media and publish it as v7.
///
/// `source` is only read. The source metadata banks are selected with the v5
/// codec directly; mounting is intentionally avoided because it can publish a
/// provisioning envelope into a legacy volume. Both target header sectors
/// must be zero. The target may be left partially written after an I/O error,
/// and must be disposable. The caller owns backups and must not treat this as
/// rollback-atomic. `volume` is caller-owned working storage and remains fenced
/// unless the generation-0 header is fully flushed.
pub fn upgrade_v5_to_v7(
    source: &mut impl Disk,
    target: &mut impl Disk,
    volume: &mut Volume7,
    expected_lineage: [u8; 16],
) -> Result<(), Error> {
    volume.fenced = true;
    volume.clear();

    match upgrade_inner(source, target, volume, expected_lineage) {
        Ok(header) => {
            volume.header = header;
            volume.fenced = false;
            Ok(())
        }
        Err(error) => {
            volume.clear();
            Err(error)
        }
    }
}

fn upgrade_inner(
    source: &mut impl Disk,
    target: &mut impl Disk,
    volume: &mut Volume7,
    expected_lineage: [u8; 16],
) -> Result<Header7, Error> {
    if expected_lineage == [0; 16] {
        return Err(Error::Invalid);
    }

    let metadata = select_source(source)?;
    let envelope_lineage = crate::provision::lineage(source)?;
    if envelope_lineage.is_some_and(|lineage| lineage != expected_lineage) {
        return Err(Error::Lineage);
    }
    let recovery = metadata.recovery.as_ref();
    if recovery.is_some_and(|recovery| recovery.lineage != expected_lineage) {
        return Err(Error::Lineage);
    }
    require_blank_target(target)?;

    let epoch = recovery.map_or(1, |recovery| recovery.epoch);
    volume.header = Header7 {
        lineage: expected_lineage,
        epoch,
        sequence: metadata.sequence,
        next: metadata.next,
        generation: 0,
        nodes_checksum: 0,
        map_checksum: 0,
        receipts_checksum: 0,
    };

    convert_live_nodes(source, &metadata.nodes, volume)?;
    convert_records(source, &metadata.nodes, recovery, volume)?;

    validate_generation(
        &volume.header,
        &volume.nodes,
        &volume.records,
        &volume.map,
        &mut volume.validation,
    )?;
    volume.header.encode()?;
    for node in &volume.nodes {
        node.encode()?;
    }
    for record in volume.records.iter().flatten() {
        record.encode()?;
    }

    stage_payloads(source, target, &metadata.nodes, recovery, volume)?;
    target.flush()?;

    let (nodes_checksum, map_checksum, receipts_checksum) = write_generation_zero(target, volume)?;
    target.flush()?;

    let published = Header7 {
        nodes_checksum,
        map_checksum,
        receipts_checksum,
        ..volume.header
    };
    let bytes = published.encode()?;
    target.write(header_sector(0), &bytes)?;
    target.flush()?;
    Ok(published)
}

fn select_source(source: &mut impl Disk) -> Result<Metadata, Error> {
    let left = Metadata::read(source, 0);
    let right = Metadata::read(source, 1);
    for result in [&left, &right] {
        if let Err(error) = result
            && !matches!(error, Error::Empty | Error::Corrupt)
        {
            return Err(*error);
        }
    }

    match (left, right) {
        (Ok(left), Ok(right)) if left.sequence == right.sequence => {
            let records_agree = left.recovery.as_ref().map(|r| r.encode())
                == right.recovery.as_ref().map(|r| r.encode());
            if left.bytes() != right.bytes() || left.next != right.next || !records_agree {
                return Err(Error::Corrupt);
            }
            Ok(left)
        }
        (Ok(left), Ok(right)) if left.sequence > right.sequence => Ok(left),
        (Ok(_), Ok(right)) => Ok(right),
        (Ok(metadata), Err(_)) | (Err(_), Ok(metadata)) => Ok(metadata),
        (Err(Error::Empty), Err(Error::Empty)) => Err(Error::Empty),
        _ => Err(Error::Corrupt),
    }
}

fn require_blank_target(target: &mut impl Disk) -> Result<(), Error> {
    for generation in 0..2 {
        let mut bytes = [0; 512];
        target.read(header_sector(generation), &mut bytes)?;
        if bytes != [0; 512] {
            return Err(Error::Exists);
        }
    }
    Ok(())
}

fn convert_live_nodes(
    source: &mut impl Disk,
    source_nodes: &[Node; crate::OBJECTS],
    target: &mut Volume7,
) -> Result<(), Error> {
    for (slot, old) in source_nodes.iter().enumerate() {
        if old.kind == Kind::Empty {
            continue;
        }
        let mut name = [0; crate::format7::NAME_BYTES];
        name[..old.name().len()].copy_from_slice(old.name());
        let mut node = crate::format7::Node7 {
            id: old.id,
            parent: old.parent,
            version: old.version,
            length: u32::from(old.length),
            kind: old.kind,
            space: old.space,
            extents_used: 0,
            extents: [Extent::new(0, 0); MAX_EXTENTS],
            name_length: old.name().len() as u8,
            name,
            payload_crc32: 0,
        };
        if old.kind == Kind::File {
            let bytes = storage::read_data(source, slot, old.bank)?;
            let length = usize::from(old.length);
            let checksum = crc(&bytes[..length]);
            if checksum != old.checksum {
                return Err(Error::Corrupt);
            }
            let plan = plan_payload(&target.map, length)?;
            reserve_plan(&mut target.map, &plan);
            node.extents_used = plan.used as u8;
            node.extents = plan.runs;
            node.payload_crc32 = payload_crc(&bytes[..length]);
        }
        target.nodes[slot] = node;
    }
    Ok(())
}

fn convert_records(
    source: &mut impl Disk,
    source_nodes: &[Node; crate::OBJECTS],
    recovery: Option<&crate::recovery::Recovery>,
    target: &mut Volume7,
) -> Result<(), Error> {
    let Some(recovery) = recovery else {
        return Ok(());
    };

    for (index, old) in recovery.records.iter().enumerate() {
        let Some(old) = old else {
            continue;
        };
        let (workspace, instance) = old.namespace.ok_or(Error::Unsupported)?;
        let (state, admission_number, terminal, prevention) = match old.admission {
            None => (RecordState::DirectCommitted, 0, old.receipt.committed, None),
            Some(admission) => match admission.state {
                AdmissionState::Admitted => (RecordState::Admitted, admission.number, 0, None),
                AdmissionState::Cancelled => (
                    RecordState::Cancelled,
                    admission.number,
                    admission.terminal,
                    Some(map_prevention(admission.prevention)?),
                ),
                AdmissionState::Committed => (
                    RecordState::AdmittedCommitted,
                    admission.number,
                    admission.terminal,
                    None,
                ),
            },
        };
        let length = usize::from(old.receipt.length);
        let bytes = &old.bytes[..length];
        let alias = if matches!(
            state,
            RecordState::DirectCommitted | RecordState::AdmittedCommitted
        ) {
            current_snapshot_extents(source, source_nodes, old, bytes, target)?
        } else {
            None
        };
        let (extents_used, extents) = if let Some(alias) = alias {
            alias
        } else {
            let plan = plan_payload(&target.map, length)?;
            reserve_plan(&mut target.map, &plan);
            (plan.used as u8, plan.runs)
        };
        target.records[index] = Some(Record7 {
            subject: old.subject,
            workspace,
            object: old.receipt.id,
            instance,
            retry_epoch: old.receipt.retry.epoch,
            retry_key: old.receipt.retry.key,
            previous: old.receipt.previous,
            committed: old.receipt.committed,
            admission_number,
            terminal,
            length: length as u32,
            payload_crc32: payload_crc(bytes),
            state,
            prevention,
            extents_used,
            extents,
        });
    }
    Ok(())
}

fn current_snapshot_extents(
    source: &mut impl Disk,
    source_nodes: &[Node; crate::OBJECTS],
    record: &crate::recovery::Record,
    snapshot: &[u8],
    target: &Volume7,
) -> Result<Option<(u8, [Extent; MAX_EXTENTS])>, Error> {
    let Some((slot, live)) = source_nodes
        .iter()
        .enumerate()
        .find(|(_, node)| node.kind != Kind::Empty && node.id == record.receipt.id)
    else {
        return Ok(None);
    };
    if live.version != record.receipt.committed {
        return Ok(None);
    }
    if live.kind != Kind::File || usize::from(live.length) != snapshot.len() {
        return Err(Error::Corrupt);
    }

    let bytes = storage::read_data(source, slot, live.bank)?;
    let length = usize::from(live.length);
    if crc(&bytes[..length]) != live.checksum || bytes[..length] != snapshot[..] {
        return Err(Error::Corrupt);
    }

    let upgraded = target.nodes[slot];
    if upgraded.kind != Kind::File
        || upgraded.length as usize != length
        || upgraded.payload_crc32 != payload_crc(snapshot)
    {
        return Err(Error::Corrupt);
    }
    Ok(Some((upgraded.extents_used, upgraded.extents)))
}

fn map_prevention(reason: Option<PreventionReason>) -> Result<PreventionReason, Error> {
    match reason {
        Some(PreventionReason::Unknown) => Ok(PreventionReason::Unknown),
        Some(PreventionReason::Requested) => Ok(PreventionReason::Requested),
        Some(PreventionReason::VersionConflict) => Ok(PreventionReason::VersionConflict),
        Some(PreventionReason::AuthorityLost) => Ok(PreventionReason::AuthorityLost),
        None => Err(Error::Corrupt),
    }
}

fn stage_payloads(
    source: &mut impl Disk,
    target: &mut impl Disk,
    source_nodes: &[Node; crate::OBJECTS],
    recovery: Option<&crate::recovery::Recovery>,
    volume: &Volume7,
) -> Result<(), Error> {
    for (slot, old) in source_nodes.iter().enumerate() {
        if old.kind != Kind::File {
            continue;
        }
        let bytes = storage::read_data(source, slot, old.bank)?;
        let length = usize::from(old.length);
        if crc(&bytes[..length]) != old.checksum {
            return Err(Error::Corrupt);
        }
        let new = volume.nodes[slot];
        let plan = PayloadPlan {
            runs: new.extents,
            used: usize::from(new.extents_used),
        };
        write_payload(target, &plan, &bytes[..length])?;
    }

    if let Some(recovery) = recovery {
        for (index, old) in recovery.records.iter().enumerate() {
            let Some(old) = old else {
                continue;
            };
            let new = volume.records[index].ok_or(Error::Corrupt)?;
            if aliases_live_payload(&new, &volume.nodes) {
                continue;
            }
            let length = usize::from(old.receipt.length);
            let plan = PayloadPlan {
                runs: new.extents,
                used: usize::from(new.extents_used),
            };
            write_payload(target, &plan, &old.bytes[..length])?;
        }
    }
    Ok(())
}

fn aliases_live_payload(record: &Record7, nodes: &[crate::format7::Node7; NODES]) -> bool {
    if !matches!(
        record.state,
        RecordState::DirectCommitted | RecordState::AdmittedCommitted
    ) {
        return false;
    }
    nodes.iter().any(|node| {
        node.kind == Kind::File
            && node.id == record.object
            && node.version == record.committed
            && node.length == record.length
            && node.payload_crc32 == record.payload_crc32
            && node.extents_used == record.extents_used
            && node.extents == record.extents
    })
}

fn write_generation_zero(disk: &mut impl Disk, volume: &Volume7) -> Result<(u32, u32, u32), Error> {
    let mut block = [0u8; 512];
    let mut nodes_checksum = !0u32;
    let base = nodes_sector(0);
    for sector in 0..NODES_SECTORS {
        block.fill(0);
        for offset in 0..512 / NODE_BYTES {
            let index = sector as usize * (512 / NODE_BYTES) + offset;
            let encoded = volume.nodes[index].encode()?;
            let at = offset * NODE_BYTES;
            block[at..at + NODE_BYTES].copy_from_slice(&encoded);
        }
        crc_update(&mut nodes_checksum, &block);
        disk.write(base + sector, &block)?;
    }

    let mut map_checksum = !0u32;
    let base = map_sector(0);
    for sector in 0..MAP_SECTORS {
        block.fill(0);
        for offset in 0..512 / 8 {
            let index = sector as usize * (512 / 8) + offset;
            block[offset * 8..offset * 8 + 8].copy_from_slice(&volume.map[index].to_le_bytes());
        }
        crc_update(&mut map_checksum, &block);
        disk.write(base + sector, &block)?;
    }

    let mut receipt_bytes = [0u8; RECEIPT_BLOCK_BYTES];
    for (index, record) in volume.records.iter().enumerate() {
        if let Some(record) = record {
            let at = index * RECORD_BYTES;
            receipt_bytes[at..at + RECORD_BYTES].copy_from_slice(&record.encode()?);
        }
    }
    let mut receipts_checksum = !0u32;
    let base = receipts_sector(0);
    for sector in 0..RECEIPTS_SECTORS {
        let at = sector as usize * 512;
        block.copy_from_slice(&receipt_bytes[at..at + 512]);
        crc_update(&mut receipts_checksum, &block);
        disk.write(base + sector, &block)?;
    }
    Ok((!nodes_checksum, !map_checksum, !receipts_checksum))
}
