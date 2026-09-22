// SPDX-License-Identifier: Apache-2.0
//! Structural checks spanning one decoded v7 generation.

use super::{Header7, MAP_WORDS, NODES, Node7, RETAINED, Record7, RecordState};
use crate::checksum::crc;
use crate::extent::Extent;
use crate::{Error, Kind};

const ROOT_NAMES: [&[u8]; 4] = [b"system", b"data", b"config", b"workspaces"];

/// Validate namespace, retained identities and allocation ownership for a
/// decoded generation. This does not validate header-region checksums or read
/// payload bytes. The caller supplies the bitmap workspace; it is cleared before
/// use and contains partial results if validation returns an error.
///
/// Map bits use the `FreeSpace` convention: one means allocated. Every set bit
/// must be owned by a live file or a retained candidate snapshot. The sole
/// permitted overlap is one committed receipt exactly aliasing its current live
/// file version.
pub fn validate_generation(
    header: &Header7,
    nodes: &[Node7; NODES],
    receipts: &[Option<Record7>; RETAINED],
    map: &[u64; MAP_WORDS],
    scratch: &mut [u64; MAP_WORDS],
) -> Result<(), Error> {
    scratch.fill(0);
    header.validate()?;
    validate_nodes(header, nodes)?;
    validate_receipts(header, nodes, receipts)?;

    for node in nodes {
        if node.kind == Kind::File && *node != Node7::EMPTY {
            mark_runs(scratch, node.runs())?;
        }
    }
    for (slot, receipt) in receipts.iter().enumerate() {
        let Some(receipt) = receipt else {
            continue;
        };
        if receipt.length == 0 && receipt.payload_crc32 != crc(&[]) {
            return Err(Error::Corrupt);
        }
        let aliases_current = aliases_current_file(receipt, nodes);
        if !aliases_current {
            mark_runs(scratch, receipt.runs())?;
        }
        // A repeated exact alias cannot be treated as another owner. Global
        // sequence uniqueness also rules it out, but keep the ownership rule
        // local to the exception that permits the first alias.
        if aliases_current
            && receipts[..slot]
                .iter()
                .flatten()
                .any(|prior| prior.object == receipt.object && aliases_current_file(prior, nodes))
        {
            return Err(Error::Corrupt);
        }
    }

    if scratch[..] != map[..] {
        return Err(Error::Corrupt);
    }
    Ok(())
}

fn validate_nodes(header: &Header7, nodes: &[Node7; NODES]) -> Result<(), Error> {
    let mut roots = [false; 4];
    for (index, node) in nodes.iter().enumerate() {
        node.validate()?;
        if *node == Node7::EMPTY {
            continue;
        }
        if node.kind == Kind::File && node.length == 0 && node.payload_crc32 != crc(&[]) {
            return Err(Error::Corrupt);
        }
        if node.id >= header.next || node.version > header.sequence {
            return Err(Error::Corrupt);
        }
        if nodes[..index].iter().any(|prior| {
            *prior != Node7::EMPTY
                && (prior.id == node.id
                    || (prior.parent == node.parent && prior.name() == node.name()))
        }) {
            return Err(Error::Corrupt);
        }
        if node.parent == 0 {
            let Some(root_index) = node.id.checked_sub(1).map(|id| id as usize) else {
                return Err(Error::Corrupt);
            };
            if root_index >= ROOT_NAMES.len()
                || node.kind != Kind::Directory
                || node.space != node.id as u8
                || node.name() != ROOT_NAMES[root_index]
            {
                return Err(Error::Corrupt);
            }
            roots[root_index] = true;
        }
    }
    if roots.iter().any(|present| !present) {
        return Err(Error::Corrupt);
    }

    for node in nodes {
        if *node == Node7::EMPTY || node.parent == 0 {
            continue;
        }
        let parent_index = find_node(nodes, node.parent).ok_or(Error::Corrupt)?;
        let parent = &nodes[parent_index];
        if parent.kind != Kind::Directory || parent.space != node.space {
            return Err(Error::Corrupt);
        }

        let mut ancestor_id = node.parent;
        let mut reached_root = false;
        for _ in 0..NODES {
            if ancestor_id == 0 {
                reached_root = true;
                break;
            }
            if ancestor_id == node.id {
                return Err(Error::Corrupt);
            }
            let ancestor = &nodes[find_node(nodes, ancestor_id).ok_or(Error::Corrupt)?];
            ancestor_id = ancestor.parent;
        }
        if !reached_root && ancestor_id == 0 {
            reached_root = true;
        }
        if !reached_root {
            return Err(Error::Corrupt);
        }
    }
    Ok(())
}

fn find_node(nodes: &[Node7; NODES], id: u32) -> Option<usize> {
    nodes
        .iter()
        .position(|node| *node != Node7::EMPTY && node.id == id)
}

fn validate_receipts(
    header: &Header7,
    nodes: &[Node7; NODES],
    receipts: &[Option<Record7>; RETAINED],
) -> Result<(), Error> {
    for (index, slot) in receipts.iter().enumerate() {
        let Some(record) = slot else {
            continue;
        };
        record.validate(header.sequence, header.next)?;
        if record.retry_epoch != header.epoch {
            return Err(Error::Corrupt);
        }
        validate_receipt_target(record, nodes)?;
        for prior in receipts[..index].iter().flatten() {
            if same_retry_identity(record, prior)
                || share_event_sequence(record, prior)
                || !same_object_history_is_consistent(record, prior)
            {
                return Err(Error::Corrupt);
            }
        }
    }
    Ok(())
}

fn validate_receipt_target(record: &Record7, nodes: &[Node7; NODES]) -> Result<(), Error> {
    let Some(index) = find_node(nodes, record.object) else {
        return Ok(());
    };
    let node = &nodes[index];
    if node.kind != Kind::File || node.version < record.previous {
        return Err(Error::Corrupt);
    }
    if matches!(record.state, RecordState::Admitted | RecordState::Cancelled)
        && node.version > record.previous
        && node.version <= record.admission_number
    {
        return Err(Error::Corrupt);
    }
    if matches!(
        record.state,
        RecordState::DirectCommitted | RecordState::AdmittedCommitted
    ) {
        if node.version < record.committed {
            return Err(Error::Corrupt);
        }
        if node.version == record.committed && !matches_committed_content(record, node) {
            return Err(Error::Corrupt);
        }
    }
    Ok(())
}

fn same_retry_identity(left: &Record7, right: &Record7) -> bool {
    left.subject == right.subject
        && left.workspace == right.workspace
        && left.retry_epoch == right.retry_epoch
        && left.retry_key == right.retry_key
}

fn share_event_sequence(left: &Record7, right: &Record7) -> bool {
    let left_events = [left.admission_number, left.terminal, left.committed];
    let right_events = [right.admission_number, right.terminal, right.committed];
    left_events
        .iter()
        .any(|left| *left != 0 && right_events.contains(left))
}

fn same_object_history_is_consistent(left: &Record7, right: &Record7) -> bool {
    if left.object != right.object {
        return true;
    }

    let left_observation = version_observation_sequence(left);
    let right_observation = version_observation_sequence(right);
    let observations_are_monotonic = match left_observation.cmp(&right_observation) {
        core::cmp::Ordering::Less => left.previous <= right.previous,
        core::cmp::Ordering::Equal => false,
        core::cmp::Ordering::Greater => right.previous <= left.previous,
    };
    if !observations_are_monotonic {
        return false;
    }
    if !committed_admission_is_unchanged(left, right)
        || !committed_admission_is_unchanged(right, left)
    {
        return false;
    }

    if let (Some((_, left_committed)), Some((right_previous, right_committed))) =
        (committed_version(left), committed_version(right))
    {
        let commits_are_monotonic = if left_committed < right_committed {
            right_previous >= left_committed
        } else {
            left.previous >= right_committed
        };
        if !commits_are_monotonic {
            return false;
        }
    }

    if let Some((_, committed)) = committed_version(left)
        && right_observation > committed
        && right.previous < committed
    {
        return false;
    }
    if let Some((_, committed)) = committed_version(right)
        && left_observation > committed
        && left.previous < committed
    {
        return false;
    }
    true
}

fn committed_admission_is_unchanged(committing: &Record7, other: &Record7) -> bool {
    if committing.state != RecordState::AdmittedCommitted {
        return true;
    }
    let event = version_observation_sequence(other);
    if committing.admission_number < event && event < committing.committed {
        other.previous == committing.previous
    } else {
        true
    }
}

fn version_observation_sequence(record: &Record7) -> u64 {
    if record.state == RecordState::DirectCommitted {
        record.committed
    } else {
        record.admission_number
    }
}

fn committed_version(record: &Record7) -> Option<(u64, u64)> {
    matches!(
        record.state,
        RecordState::DirectCommitted | RecordState::AdmittedCommitted
    )
    .then_some((record.previous, record.committed))
}

fn aliases_current_file(record: &Record7, nodes: &[Node7; NODES]) -> bool {
    if !matches!(
        record.state,
        RecordState::DirectCommitted | RecordState::AdmittedCommitted
    ) {
        return false;
    }
    let Some(index) = find_node(nodes, record.object) else {
        return false;
    };
    let node = &nodes[index];
    node.version == record.committed && matches_committed_content(record, node)
}

fn matches_committed_content(record: &Record7, node: &Node7) -> bool {
    node.kind == Kind::File
        && node.length == record.length
        && node.payload_crc32 == record.payload_crc32
        && node.extents_used == record.extents_used
        && node.extents == record.extents
}

fn mark_runs(scratch: &mut [u64; MAP_WORDS], runs: &[Extent]) -> Result<(), Error> {
    for run in runs {
        for sector in run.start..run.end() {
            let word = &mut scratch[sector as usize / 64];
            let bit = 1u64 << (sector % 64);
            if *word & bit != 0 {
                return Err(Error::Corrupt);
            }
            *word |= bit;
        }
    }
    Ok(())
}
