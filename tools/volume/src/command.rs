// SPDX-License-Identifier: Apache-2.0
//! The four volume operations this tool exposes, all against a real image file.
use std::path::Path;

use rustic_fs::{DATA_SECTORS, Kind, Node6, VOLUME_SECTORS, Volume, mount6, provision6, upgrade6};

use crate::disk::FileDisk;

/// The smallest image this tool will touch: a volume's structures plus payload.
const IMAGE_SECTORS: u64 = VOLUME_SECTORS;
/// The fixture `seed` writes: a directory and a file whose bytes a v5 reader
/// can confirm, so a later migration is checked against an independent source.
const SEEDED: &[u8] = b"a v5 record";

pub(crate) fn provision(image: &Path, lineage: &str) -> Result<String, String> {
    let mut disk = FileDisk::create(image, IMAGE_SECTORS)?;
    let volume = provision6(&mut disk, parse_lineage(lineage)?)
        .map_err(|error| format!("provision refused: {error:?}"))?;
    Ok(format!(
        "{{\"sequence\":{},\"active\":{},\"free_sectors\":{},\"payload_sectors\":{}}}",
        volume.header.sequence,
        volume.header.active,
        volume.free_sectors(),
        DATA_SECTORS
    ))
}

pub(crate) fn seed(image: &Path) -> Result<String, String> {
    let mut disk = FileDisk::create(image, IMAGE_SECTORS)?;
    let mut volume =
        Volume::initialize(&mut disk).map_err(|error| format!("seed refused: {error:?}"))?;
    let directory = volume
        .create(&mut disk, 4, b"project", Kind::Directory)
        .map_err(|error| format!("seed directory refused: {error:?}"))?;
    let notes = volume
        .create(&mut disk, directory.id, b"notes.txt", Kind::File)
        .map_err(|error| format!("seed file refused: {error:?}"))?;
    let notes = volume
        .replace(&mut disk, notes.id, notes.version, SEEDED)
        .map_err(|error| format!("seed content refused: {error:?}"))?;
    let empty = volume
        .create(&mut disk, 4, b"empty.bin", Kind::File)
        .map_err(|error| format!("seed file refused: {error:?}"))?;
    Ok(format!(
        "{{\"sequence\":{},\"directory\":{{\"id\":{},\"version\":{}}},\
         \"notes\":{{\"id\":{},\"version\":{},\"length\":{}}},\"empty\":{{\"id\":{}}}}}",
        volume.sequence(),
        directory.id,
        directory.version,
        notes.id,
        notes.version,
        SEEDED.len(),
        empty.id
    ))
}

/// Place a host file in a v6 image, reusing an existing record with the same
/// parent and name so a fixture is written once and rewritten in place.
pub(crate) fn write(
    image: &Path,
    parent: &str,
    name: &str,
    source: &Path,
) -> Result<String, String> {
    let bytes = std::fs::read(source)
        .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
    let parent: u32 = parent
        .parse()
        .map_err(|_| "parent must be a node id".to_owned())?;
    if name.is_empty() || name.len() > 32 || !name.is_ascii() {
        return Err("name must be 1..=32 ASCII bytes".to_owned());
    }
    let mut disk = open(image)?;
    let mut volume = mount6(&mut disk).map_err(|error| format!("mount refused: {error:?}"))?;
    let directory = volume
        .node(parent)
        .ok_or_else(|| format!("no node {parent}"))?;
    if directory.kind != Kind::Directory {
        return Err(format!("node {parent} is not a directory"));
    }
    let existing = volume.nodes.iter().position(|node| {
        node.kind == Kind::File && node.parent == parent && node.name() == name.as_bytes()
    });
    let slot = match existing {
        Some(slot) => slot,
        None => {
            let slot = volume
                .nodes
                .iter()
                .position(|node| node.kind == Kind::Empty)
                .ok_or("no free node slot")?;
            let id = volume.nodes.iter().map(|node| node.id).max().unwrap_or(0) + 1;
            let mut record = Node6::EMPTY;
            record.id = id;
            record.parent = parent;
            record.kind = Kind::File;
            record.version = 1;
            record.name[..name.len()].copy_from_slice(name.as_bytes());
            record.name_length = name.len() as u8;
            volume.nodes[slot] = record;
            slot
        }
    };
    let expected = volume.nodes[slot].version;
    let version = volume
        .write_file(&mut disk, slot, expected, &bytes)
        .map_err(|error| format!("write refused: {error:?}"))?;
    let node = &volume.nodes[slot];
    let extents: Vec<String> = node
        .runs()
        .iter()
        .map(|run| format!("[{},{}]", run.start, run.sectors))
        .collect();
    Ok(format!(
        "{{\"id\":{},\"version\":{version},\"length\":{},\"extents\":[{}]}}",
        node.id,
        bytes.len(),
        extents.join(",")
    ))
}

pub(crate) fn migrate(image: &Path, lineage: &str) -> Result<String, String> {
    let mut disk = open(image)?;
    let upgraded = upgrade6(&mut disk, parse_lineage(lineage)?)
        .map_err(|error| format!("migration refused: {error:?}"))?;
    let report = upgraded.report;
    Ok(format!(
        "{{\"source_sequence\":{},\"next\":{},\"files\":{},\"directories\":{},\"bytes\":{}}}",
        report.source_sequence, report.next, report.files, report.directories, report.bytes
    ))
}

pub(crate) fn report(image: &Path) -> Result<String, String> {
    let mut disk = open(image)?;
    let volume = mount6(&mut disk).map_err(|error| format!("mount refused: {error:?}"))?;
    let nodes: Vec<String> = volume
        .nodes
        .iter()
        .filter(|node| node.kind != Kind::Empty)
        .map(|node| {
            let extents: Vec<String> = node
                .runs()
                .iter()
                .map(|run| format!("[{},{}]", run.start, run.sectors))
                .collect();
            format!(
                "{{\"id\":{},\"parent\":{},\"version\":{},\"length\":{},\"kind\":\"{}\",\
                 \"space\":{},\"name\":\"{}\",\"extents\":[{}]}}",
                node.id,
                node.parent,
                node.version,
                node.length,
                kind_name(node.kind),
                node.space,
                escape(node.name()),
                extents.join(",")
            )
        })
        .collect();
    Ok(format!(
        "{{\"sequence\":{},\"active\":{},\"objects\":{},\"used_sectors\":{},\"free_sectors\":{},\
         \"lineage\":\"{}\",\"epoch\":{},\"receipts\":{},\"nodes\":[{}]}}",
        volume.header.sequence,
        volume.header.active,
        volume.header.objects,
        DATA_SECTORS - volume.free_sectors(),
        volume.free_sectors(),
        hex(&volume.receipts.lineage()),
        volume.receipts.epoch(),
        volume.receipts.len(),
        nodes.join(",")
    ))
}

fn open(image: &Path) -> Result<FileDisk, String> {
    let disk = FileDisk::open(image)?;
    if disk.sectors() < IMAGE_SECTORS {
        return Err(format!(
            "{} is {} sectors; a volume is {IMAGE_SECTORS}",
            image.display(),
            disk.sectors()
        ));
    }
    Ok(disk)
}

pub(crate) fn parse_lineage(value: &str) -> Result<[u8; 16], String> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("lineage must be 32 hex characters".to_owned());
    }
    let mut lineage = [0; 16];
    for (index, slot) in lineage.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| "lineage must be 32 hex characters".to_owned())?;
    }
    if lineage == [0; 16] {
        return Err("lineage must not be all zero".to_owned());
    }
    Ok(lineage)
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Empty => "empty",
        Kind::File => "file",
        Kind::Directory => "directory",
    }
}

fn escape(name: &[u8]) -> String {
    name.iter()
        .map(|byte| match byte {
            b'"' => "\\\"".to_owned(),
            b'\\' => "\\\\".to_owned(),
            0x20..=0x7e => char::from(*byte).to_string(),
            _ => format!("\\u{:04x}", byte),
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
