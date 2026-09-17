// SPDX-License-Identifier: Apache-2.0
//! Deliberate v5 -> v6 volume upgrade (#51).
//!
//! The two layouts overlap: the v5 volume keeps its header at sector 8 and its
//! banked file data from sector 32, while a v6 volume keeps its header, node
//! tables, maps and receipts in sectors 8..205. An in-place upgrade therefore
//! reads every v5 record first, copies each file's payload into the free v6
//! payload region (sector 205 and above, outside the v5 layout) and only then
//! publishes the v6 structures with one flush. Until that flush the v5 header is
//! untouched, so a failure while staging leaves a mountable v5 volume.
//!
//! Two states cannot be reconstructed and are refused instead of being dropped:
//!
//! * a retained v5 recovery record carries the original file bytes as evidence
//!   and the v6 receipt table holds no such snapshot yet;
//! * a v5 provisioning envelope with a different lineage is another volume's
//!   identity, not this migration's input.
//!
//! The caller owns durability and the backup. A publish torn between the v6
//! structure writes and the header write leaves a volume whose v5 header still
//! describes data that the v6 structure writes have already overwritten; the
//! migration is a one-way operation that must run against a copy.
//!
//! Exactly one layout is left mountable: the v5 volume keeps a second metadata
//! bank whose header sits at sector 13, outside the v6 structures, so the
//! upgrade clears it before publishing. Otherwise a v5 mount could still find
//! the old bank after the migration and publish a v5 header over the v6 one.

use crate::format6::Node6;
use crate::{Disk, Error, Kind, MAX_FILE, OBJECTS, Volume, Volume6};

/// What the upgrade moved, so the caller can seed identity allocation and record
/// evidence without re-reading the old layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    /// Sequence of the v5 publication the upgrade read.
    pub source_sequence: u64,
    /// The v5 identity allocator, which v6 does not carry: the caller derives new
    /// ids from the migrated records and this watermark.
    pub next: u32,
    /// Live records migrated, by kind, and how many file bytes they carry.
    pub files: u32,
    pub directories: u32,
    pub bytes: u64,
}

/// The mounted v6 volume and the account of what it inherited.
pub struct UpgradedVolume {
    pub volume: Volume6,
    pub report: Report,
}

/// Upgrade a mounted v5 volume in place, preserving identity, names, versions,
/// kinds, spaces and file bytes. `lineage` must match the v5 envelope when the
/// volume carries one.
pub fn upgrade(disk: &mut impl Disk, lineage: [u8; 16]) -> Result<UpgradedVolume, Error> {
    if crate::volume6::mount(disk).is_ok() {
        return Err(Error::Exists);
    }
    let source = Volume::mount(disk)?;
    if let Some(recovery) = &source.metadata.recovery {
        if recovery.lineage != lineage {
            return Err(Error::Lineage);
        }
        if recovery.records.iter().any(Option::is_some) {
            return Err(Error::Unsupported);
        }
    }
    let nodes = source.metadata.nodes;
    validate(&nodes)?;

    let mut target = crate::volume6::blank(lineage)?;
    let mut report = Report {
        source_sequence: source.sequence(),
        next: source.metadata.next,
        files: 0,
        directories: 0,
        bytes: 0,
    };
    for (slot, node) in nodes.iter().enumerate() {
        if node.kind == Kind::Empty {
            continue;
        }
        let mut record = Node6::EMPTY;
        record.id = node.id;
        record.parent = node.parent;
        record.version = node.version;
        record.kind = node.kind;
        record.space = node.space;
        let name = node.name();
        record.name[..name.len()].copy_from_slice(name);
        record.name_length = name.len() as u8;
        target.nodes[slot] = record;
        match node.kind {
            Kind::File => {
                let length = usize::from(node.length);
                let payload = crate::storage::read_data(disk, slot, node.bank)?;
                // Payload sectors live above the v5 layout, so staging cannot
                // disturb the records or bytes still being read.
                target.stage_bytes(disk, slot, &payload[..length])?;
                report.files += 1;
                report.bytes += length as u64;
            }
            Kind::Directory => report.directories += 1,
            Kind::Empty => {}
        }
    }
    // Both v5 metadata banks must stop mounting before the v6 header exists:
    // the publish below covers bank 0, and bank 1 sits outside the v6 structures.
    disk.write(crate::storage::header(1), &[0; 512])?;
    disk.flush()?;
    target.flush(disk)?;
    Ok(UpgradedVolume {
        volume: target,
        report,
    })
}

/// A migration copies records as they are, so a namespace that v5 tolerated but
/// cannot be read consistently is refused before a single payload sector is
/// staged.
fn validate(nodes: &[crate::Node; OBJECTS]) -> Result<(), Error> {
    for (slot, node) in nodes.iter().enumerate() {
        if node.kind == Kind::Empty {
            continue;
        }
        if node.id == 0 || node.name().is_empty() || usize::from(node.length) > MAX_FILE {
            return Err(Error::Corrupt);
        }
        if nodes[..slot].iter().any(|other| other.id == node.id) {
            return Err(Error::Corrupt);
        }
        if node.parent != 0 {
            let parent = nodes
                .iter()
                .find(|other| other.id == node.parent)
                .ok_or(Error::Corrupt)?;
            if parent.kind != Kind::Directory {
                return Err(Error::Corrupt);
            }
        }
    }
    Ok(())
}
