// SPDX-License-Identifier: Apache-2.0
//! Host volume-image commands for legacy v5/v6 images and explicit v7 fixtures.
use std::fs::File;
use std::io::Read;
use std::path::Path;

use rustic_abi::application::Manifest;
use rustic_abi::files::reference::{Resource, Workspace};
use rustic_fs::format7::{Record7, RecordState};
use rustic_fs::{
    DATA_SECTORS, Kind, Node6, PreventionReason, VOLUME_SECTORS, Volume, Volume7, WriteIdentity7,
    format7, mount6, provision6, upgrade6,
};
use sha2::{Digest, Sha256};

use crate::disk::FileDisk;

/// The smallest image this tool will touch: a volume's structures plus payload.
pub(crate) const IMAGE_SECTORS: u64 = VOLUME_SECTORS;
const V7_IMAGE_SECTORS: u64 = format7::VOLUME_SECTORS;
const V7_IMAGE_BYTES: u64 = V7_IMAGE_SECTORS * format7::SECTOR_BYTES;
/// The fixture `seed` writes: a directory and a file whose bytes a v5 reader
/// can confirm, so a later migration is checked against an independent source.
const SEEDED: &[u8] = b"a v5 record";
const V7_WORKSPACE_NAME: &[u8] = b"application";
/// Writable file `seed7 --scratch` adds for guest tracked-write harnesses.
const V7_SCRATCH_NAME: &[u8] = b"scratch.bin";

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

/// Create a fresh v7 image containing an application ELF and its manifest.
/// All host inputs are checked before the output path is exclusively created.
pub(crate) fn seed7(
    image: &Path,
    lineage: &str,
    elf_path: &Path,
    manifest_path: &Path,
    scratch: bool,
) -> Result<String, String> {
    let lineage = parse_lineage(lineage)?;
    let elf_name = source_name(elf_path, ".elf", "ELF")?;
    let manifest_name = source_name(manifest_path, ".manifest", "manifest")?;
    if elf_name == manifest_name {
        return Err("ELF and manifest names must be different".to_owned());
    }

    let elf = read_bounded(elf_path, "ELF", format7::MAX_FILE_BYTES as u64)?;
    validate_elf(&elf)?;
    let manifest = read_bounded(
        manifest_path,
        "manifest",
        rustic_abi::application::SIZE as u64,
    )?;
    if manifest.len() != rustic_abi::application::SIZE {
        return Err(format!(
            "manifest must be exactly {} bytes",
            rustic_abi::application::SIZE
        ));
    }
    let parsed =
        Manifest::parse(&manifest).map_err(|error| format!("invalid manifest: {error:?}"))?;
    if parsed.executable != elf_name {
        return Err(format!(
            "manifest names {}, but the ELF file is {elf_name}",
            parsed.executable
        ));
    }
    let elf_sha256: [u8; 32] = Sha256::digest(&elf).into();
    if parsed.artifact_sha256 != elf_sha256 {
        return Err("manifest ELF digest does not match the executable".to_owned());
    }

    let mut disk = FileDisk::create_new(image, V7_IMAGE_SECTORS)?;
    let mut volume = Volume7::EMPTY;
    volume
        .provision_into(&mut disk, lineage)
        .map_err(|error| format!("v7 provision refused: {error:?}"))?;

    let workspace = volume
        .create(&mut disk, 4, V7_WORKSPACE_NAME, Kind::Directory)
        .map_err(|error| format!("v7 workspace creation refused: {error:?}"))?;
    let elf_node = volume
        .create(&mut disk, workspace.id, elf_name.as_bytes(), Kind::File)
        .map_err(|error| format!("v7 ELF creation refused: {error:?}"))?;
    let manifest_node = volume
        .create(
            &mut disk,
            workspace.id,
            manifest_name.as_bytes(),
            Kind::File,
        )
        .map_err(|error| format!("v7 manifest creation refused: {error:?}"))?;

    let epoch = volume
        .header()
        .map_err(|error| format!("v7 header unavailable: {error:?}"))?
        .epoch;
    let elf_record = volume
        .replace_tracked(
            &mut disk,
            write_identity(workspace.id, elf_node.id, elf_node.version, epoch),
            elf_node.version,
            &elf,
        )
        .map_err(|error| format!("v7 ELF write refused: {error:?}"))?;
    let manifest_record = volume
        .replace_tracked(
            &mut disk,
            write_identity(workspace.id, manifest_node.id, manifest_node.version, epoch),
            manifest_node.version,
            &manifest,
        )
        .map_err(|error| format!("v7 manifest write refused: {error:?}"))?;

    // An empty, untracked file for guest writes, so tracked-write harnesses do
    // not replace the application pair other harnesses depend on. Creating it
    // consumes no retained record.
    let scratch = if scratch {
        let node = volume
            .create(&mut disk, workspace.id, V7_SCRATCH_NAME, Kind::File)
            .map_err(|error| format!("v7 scratch creation refused: {error:?}"))?;
        format!(
            ",\"scratch\":{{\"id\":{},\"version\":{},\"size\":0,\"resource\":\"{}\"}}",
            node.id,
            node.version,
            resource_text(lineage, workspace.id, node.id)?,
        )
    } else {
        String::new()
    };

    let workspace_text = workspace_text(lineage, workspace.id)?;
    let elf_resource = resource_text(lineage, workspace.id, elf_node.id)?;
    let manifest_resource = resource_text(lineage, workspace.id, manifest_node.id)?;
    Ok(format!(
        "{{\"lineage\":\"{}\",\"workspace\":{{\"id\":{},\"text\":\"{}\"}},\
         \"elf\":{{\"id\":{},\"version\":{},\"size\":{},\"resource\":\"{}\"}},\
         \"manifest\":{{\"id\":{},\"version\":{},\"size\":{},\"resource\":\"{}\"}}{}}}",
        hex(&lineage),
        workspace.id,
        workspace_text,
        elf_node.id,
        elf_record.committed,
        elf.len(),
        elf_resource,
        manifest_node.id,
        manifest_record.committed,
        manifest.len(),
        manifest_resource,
        scratch,
    ))
}

/// Verify a full v7 mount and report its selected metadata without write access.
pub(crate) fn report7(image: &Path) -> Result<String, String> {
    let mut disk = FileDisk::open_read_only(image)?;
    if disk.sectors() != V7_IMAGE_SECTORS || disk.bytes()? != V7_IMAGE_BYTES {
        return Err(format!(
            "{} is not an exact v7 image of {V7_IMAGE_BYTES} bytes",
            image.display()
        ));
    }
    let mut volume = Volume7::EMPTY;
    volume
        .mount_into(&mut disk)
        .map_err(|error| format!("v7 mount refused: {error:?}"))?;
    let header = *volume
        .header()
        .map_err(|error| format!("v7 header unavailable: {error:?}"))?;
    let nodes = volume
        .nodes()
        .map_err(|error| format!("v7 nodes unavailable: {error:?}"))?;
    let live_nodes: Vec<String> = nodes
        .iter()
        .filter(|node| node.kind != Kind::Empty)
        .map(|node| {
            format!(
                "{{\"id\":{},\"parent\":{},\"version\":{},\"size\":{},\
                 \"kind\":\"{}\",\"name\":\"{}\"}}",
                node.id,
                node.parent,
                node.version,
                node.length,
                kind_name(node.kind),
                escape(node.name()),
            )
        })
        .collect();
    let free_sectors = volume
        .free_sectors()
        .map_err(|error| format!("v7 map unavailable: {error:?}"))?;
    let recovered = volume
        .recovered_from_header()
        .map_err(|error| format!("v7 header state unavailable: {error:?}"))?;
    let records: Vec<String> = volume
        .retained_records()
        .map_err(|error| format!("v7 records unavailable: {error:?}"))?
        .iter()
        .enumerate()
        .filter_map(|(slot, record)| record.map(|record| record_json(slot, &record)))
        .collect();
    Ok(format!(
        "{{\"lineage\":\"{}\",\"sequence\":{},\"epoch\":{},\"generation\":{},\
         \"next\":{},\"objects\":{},\"free_sectors\":{},\"recovered\":{},\
         \"workspace\":{{\"id\":4,\"text\":\"{}\"}},\"nodes\":[{}],\"records\":[{}]}}",
        hex(&header.lineage),
        header.sequence,
        header.epoch,
        header.generation,
        header.next,
        live_nodes.len(),
        free_sectors,
        recovered,
        workspace_text(header.lineage, 4)?,
        live_nodes.join(","),
        records.join(","),
    ))
}

/// One verified retained record, for comparison with an independent reader.
fn record_json(slot: usize, record: &Record7) -> String {
    let state = match record.state {
        RecordState::DirectCommitted => "direct_committed",
        RecordState::Admitted => "admitted",
        RecordState::Cancelled => "cancelled",
        RecordState::AdmittedCommitted => "admitted_committed",
    };
    let cause = match record.prevention {
        None => "null",
        Some(PreventionReason::Unknown) => "\"unknown\"",
        Some(PreventionReason::Requested) => "\"requested\"",
        Some(PreventionReason::VersionConflict) => "\"version_conflict\"",
        Some(PreventionReason::AuthorityLost) => "\"authority_lost\"",
    };
    format!(
        "{{\"slot\":{slot},\"state\":\"{state}\",\"cause\":{cause},\"subject\":{},\
         \"workspace\":{},\"object\":{},\"instance\":{},\"epoch\":{},\"key\":{},\
         \"previous\":{},\"committed\":{},\"admission\":{},\"terminal\":{},\"size\":{}}}",
        record.subject,
        record.workspace,
        record.object,
        record.instance,
        record.retry_epoch,
        record.retry_key,
        record.previous,
        record.committed,
        record.admission_number,
        record.terminal,
        record.length,
    )
}

fn write_identity(workspace: u32, object: u32, instance: u64, retry_epoch: u64) -> WriteIdentity7 {
    WriteIdentity7 {
        subject: 1,
        workspace,
        object,
        instance,
        retry_epoch,
        retry_key: u64::from(object),
    }
}

fn source_name(path: &Path, suffix: &str, label: &str) -> Result<String, String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{label} path needs an ASCII filename"))?;
    if !name.ends_with(suffix) || !valid_name(name.as_bytes()) {
        return Err(format!(
            "{label} filename must be a valid workspace name ending in {suffix}"
        ));
    }
    Ok(name.to_owned())
}

fn valid_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= 31
        && name != b"."
        && name != b".."
        && name
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(byte))
}

fn read_bounded(path: &Path, label: &str, max: u64) -> Result<Vec<u8>, String> {
    let file =
        File::open(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot size {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("{label} {} is not a regular file", path.display()));
    }
    if metadata.len() > max {
        return Err(format!(
            "{label} {} is {} bytes; maximum is {max}",
            path.display(),
            metadata.len()
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(max.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if bytes.len() as u64 > max || bytes.len() as u64 != metadata.len() {
        return Err(format!(
            "{label} {} changed while being read",
            path.display()
        ));
    }
    Ok(bytes)
}

fn validate_elf(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 64
        || &bytes[..4] != b"\x7fELF"
        || bytes[4] != 2
        || bytes[5] != 1
        || bytes[6] != 1
        || le_u16(bytes, 16) != 2
        || le_u16(bytes, 18) != 62
        || le_u32(bytes, 20) != 1
        || le_u16(bytes, 52) != 64
        || le_u16(bytes, 54) != 56
    {
        return Err("ELF must be a little-endian x86-64 executable".to_owned());
    }
    let phoff = usize::try_from(le_u64(bytes, 32)).map_err(|_| "ELF program table is too large")?;
    let phnum = usize::from(le_u16(bytes, 56));
    let table_bytes = phnum
        .checked_mul(56)
        .ok_or_else(|| "ELF program table is too large".to_owned())?;
    let table_end = phoff
        .checked_add(table_bytes)
        .ok_or_else(|| "ELF program table is too large".to_owned())?;
    if phnum == 0 || table_end > bytes.len() {
        return Err("ELF program table is incomplete".to_owned());
    }
    let mut has_load_segment = false;
    for header in bytes[phoff..table_end].as_chunks::<56>().0 {
        if u32::from_le_bytes(header[..4].try_into().unwrap()) == 1 {
            has_load_segment = true;
            let offset = usize::try_from(u64::from_le_bytes(header[8..16].try_into().unwrap()))
                .map_err(|_| "ELF load segment is too large")?;
            let file_size = usize::try_from(u64::from_le_bytes(header[32..40].try_into().unwrap()))
                .map_err(|_| "ELF load segment is too large")?;
            let end = offset
                .checked_add(file_size)
                .ok_or_else(|| "ELF load segment is too large".to_owned())?;
            if end > bytes.len() {
                return Err("ELF load segment exceeds the file".to_owned());
            }
        }
    }
    if !has_load_segment {
        return Err("ELF has no loadable segment".to_owned());
    }
    Ok(())
}

fn le_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn le_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn le_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

pub(crate) fn workspace_text(lineage: [u8; 16], root: u32) -> Result<String, String> {
    Workspace::new(lineage, root)
        .map(|workspace| workspace.to_string())
        .map_err(|error| format!("invalid workspace identity: {error:?}"))
}

pub(crate) fn resource_text(lineage: [u8; 16], root: u32, object: u32) -> Result<String, String> {
    let workspace = Workspace::new(lineage, root)
        .map_err(|error| format!("invalid workspace identity: {error:?}"))?;
    Resource::new(workspace, object)
        .map(|resource| resource.to_string())
        .map_err(|error| format!("invalid resource identity: {error:?}"))
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

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use rustic_abi::files::reference::{Resource, Workspace};
    use std::path::PathBuf;
    use std::str::FromStr;

    const LINEAGE: &str = "000102030405060708090a0b0c0d0e0f";

    fn fixture_elf(size: usize) -> Vec<u8> {
        assert!(size >= 120);
        let mut elf = vec![0; size];
        elf[..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2;
        elf[5] = 1;
        elf[6] = 1;
        elf[16..18].copy_from_slice(&2u16.to_le_bytes());
        elf[18..20].copy_from_slice(&62u16.to_le_bytes());
        elf[20..24].copy_from_slice(&1u32.to_le_bytes());
        elf[24..32].copy_from_slice(&0x400000u64.to_le_bytes());
        elf[32..40].copy_from_slice(&64u64.to_le_bytes());
        elf[52..54].copy_from_slice(&64u16.to_le_bytes());
        elf[54..56].copy_from_slice(&56u16.to_le_bytes());
        elf[56..58].copy_from_slice(&1u16.to_le_bytes());
        elf[64..68].copy_from_slice(&1u32.to_le_bytes());
        elf[68..72].copy_from_slice(&5u32.to_le_bytes());
        elf[72..80].copy_from_slice(&0u64.to_le_bytes());
        elf[80..88].copy_from_slice(&0x400000u64.to_le_bytes());
        elf[88..96].copy_from_slice(&0x400000u64.to_le_bytes());
        elf[96..104].copy_from_slice(&(size as u64).to_le_bytes());
        elf[104..112].copy_from_slice(&(size as u64).to_le_bytes());
        elf[112..120].copy_from_slice(&0x1000u64.to_le_bytes());
        elf
    }

    fn fixture_manifest(elf: &[u8]) -> [u8; rustic_abi::application::SIZE] {
        let mut bytes = [0; rustic_abi::application::SIZE];
        bytes[..8].copy_from_slice(b"RUSTAPP\0");
        bytes[8..10].copy_from_slice(&2u16.to_le_bytes());
        bytes[10..12].copy_from_slice(&(rustic_abi::application::SIZE as u16).to_le_bytes());
        bytes[12..16].copy_from_slice(&(rustic_abi::process::VERSION as u32).to_le_bytes());
        bytes[16..18].copy_from_slice(&rustic_abi::ipc::VERSION.to_le_bytes());
        bytes[32..55].copy_from_slice(b"org.rusticos.v7-fixture");
        bytes[64..75].copy_from_slice(b"fixture.elf");
        bytes[96..128].copy_from_slice(&Sha256::digest(elf));
        bytes
    }

    fn write_inputs(dir: &Path, elf: &[u8]) -> (PathBuf, PathBuf) {
        let elf_path = dir.join("fixture.elf");
        let manifest_path = dir.join("fixture.manifest");
        std::fs::write(&elf_path, elf).unwrap();
        std::fs::write(&manifest_path, fixture_manifest(elf)).unwrap();
        (elf_path, manifest_path)
    }

    #[test]
    fn seed7_exclusively_creates_exact_v7_image_and_remounts_exact_artifact_bytes() {
        let dir = TempDir::new();
        let image = dir.path().join("fixture.raw");
        let elf = fixture_elf(300_000);
        assert!(elf.len() > 256 * 1024);
        assert!(elf.len() <= format7::MAX_FILE_BYTES as usize);
        let (elf_path, manifest_path) = write_inputs(dir.path(), &elf);
        let manifest = fixture_manifest(&elf);

        let result = seed7(&image, LINEAGE, &elf_path, &manifest_path, false).unwrap();
        assert_eq!(std::fs::metadata(&image).unwrap().len(), V7_IMAGE_BYTES);

        let lineage = parse_lineage(LINEAGE).unwrap();
        let workspace_text = format!("ws_{LINEAGE}_00000005");
        let elf_resource_text = format!("rs_{LINEAGE}_00000005_00000006");
        let manifest_resource_text = format!("rs_{LINEAGE}_00000005_00000007");
        let expected = format!(
            "{{\"lineage\":\"{LINEAGE}\",\"workspace\":{{\"id\":5,\"text\":\"{workspace_text}\"}},\
             \"elf\":{{\"id\":6,\"version\":5,\"size\":300000,\"resource\":\"{elf_resource_text}\"}},\
             \"manifest\":{{\"id\":7,\"version\":6,\"size\":128,\"resource\":\"{manifest_resource_text}\"}}}}"
        );
        assert_eq!(result, expected);

        let workspace = Workspace::from_str(&workspace_text).unwrap();
        let elf_resource = Resource::from_str(&elf_resource_text).unwrap();
        let manifest_resource = Resource::from_str(&manifest_resource_text).unwrap();
        assert_eq!(workspace.root(), 5);
        assert_eq!(workspace.lineage(), lineage);
        assert_eq!(elf_resource.workspace(), workspace);
        assert_eq!(manifest_resource.workspace(), workspace);
        assert_eq!(elf_resource.object(), 6);
        assert_eq!(manifest_resource.object(), 7);

        let mut disk = FileDisk::open_read_only(&image).unwrap();
        let mut volume = Volume7::EMPTY;
        volume.mount_into(&mut disk).unwrap();
        assert_eq!(volume.header().unwrap().lineage, lineage);
        let mut elf_read = vec![0; elf.len()];
        let elf_count = volume
            .read_range(&mut disk, elf_resource.object(), Some(5), 0, &mut elf_read)
            .unwrap();
        assert_eq!(elf_count, elf.len());
        assert_eq!(elf_read, elf);
        let mut manifest_read = [0; rustic_abi::application::SIZE];
        let manifest_count = volume
            .read_range(
                &mut disk,
                manifest_resource.object(),
                Some(6),
                0,
                &mut manifest_read,
            )
            .unwrap();
        assert_eq!(manifest_count, manifest.len());
        assert_eq!(manifest_read, manifest);

        let report = report7(&image).unwrap();
        assert!(report.contains("\"sequence\":6"));
        assert!(report.contains(&format!("\"lineage\":\"{LINEAGE}\"")));
        assert!(report.contains("\"name\":\"fixture.elf\""));
        assert!(report.contains("\"name\":\"fixture.manifest\""));
        assert!(report.contains("\"recovered\":false"));
        assert!(
            report.contains(
                "\"records\":[{\"slot\":0,\"state\":\"direct_committed\",\"cause\":null,"
            )
        );
    }

    #[test]
    fn seed7_scratch_adds_an_empty_untracked_file_and_keeps_the_pair_and_records() {
        let dir = TempDir::new();
        let image = dir.path().join("fixture.raw");
        let elf = fixture_elf(4096);
        let (elf_path, manifest_path) = write_inputs(dir.path(), &elf);
        let result = seed7(&image, LINEAGE, &elf_path, &manifest_path, true).unwrap();
        let scratch_text = format!("rs_{LINEAGE}_00000005_00000008");
        assert!(result.ends_with(&format!(
            ",\"scratch\":{{\"id\":8,\"version\":7,\"size\":0,\"resource\":\"{scratch_text}\"}}}}"
        )));
        assert!(result.contains("\"elf\":{\"id\":6,\"version\":5,"));

        let mut disk = FileDisk::open_read_only(&image).unwrap();
        let mut volume = Volume7::EMPTY;
        volume.mount_into(&mut disk).unwrap();
        let scratch = volume.node(8).unwrap().copied().unwrap();
        assert_eq!(
            (
                scratch.kind,
                scratch.parent,
                scratch.version,
                scratch.length
            ),
            (Kind::File, 5, 7, 0)
        );
        assert_eq!(scratch.name(), V7_SCRATCH_NAME);
        let records = volume.retained_records().unwrap();
        assert_eq!(records.iter().flatten().count(), 2);
        assert!(records.iter().flatten().all(|record| record.object != 8));
    }

    #[test]
    fn seed7_rejects_manifest_for_different_executable_before_creating_image() {
        let dir = TempDir::new();
        let image = dir.path().join("fixture.raw");
        let elf = fixture_elf(300_000);
        let (elf_path, manifest_path) = write_inputs(dir.path(), &elf);
        let mut changed_elf = elf;
        *changed_elf.last_mut().unwrap() ^= 1;
        std::fs::write(&elf_path, changed_elf).unwrap();

        let error = seed7(&image, LINEAGE, &elf_path, &manifest_path, false).unwrap_err();
        assert!(error.contains("digest does not match"));
        assert!(!image.exists());
    }

    #[test]
    fn seed7_refuses_existing_files_directories_and_symlinks_without_overwriting() {
        let dir = TempDir::new();
        let (elf_path, manifest_path) = write_inputs(dir.path(), &fixture_elf(4096));

        let existing_file = dir.path().join("existing.raw");
        std::fs::write(&existing_file, b"preserve this image").unwrap();
        assert!(seed7(&existing_file, LINEAGE, &elf_path, &manifest_path, false).is_err());
        assert_eq!(
            std::fs::read(&existing_file).unwrap(),
            b"preserve this image"
        );

        let existing_dir = dir.path().join("existing-dir.raw");
        std::fs::create_dir(&existing_dir).unwrap();
        assert!(seed7(&existing_dir, LINEAGE, &elf_path, &manifest_path, false).is_err());
        assert!(existing_dir.is_dir());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let target = dir.path().join("target.raw");
            let link = dir.path().join("link.raw");
            std::fs::write(&target, b"symlink target stays intact").unwrap();
            symlink(&target, &link).unwrap();
            assert!(seed7(&link, LINEAGE, &elf_path, &manifest_path, false).is_err());
            assert_eq!(
                std::fs::read(&target).unwrap(),
                b"symlink target stays intact"
            );
            assert!(
                std::fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
    }

    #[test]
    fn seed7_rejects_an_oversized_elf_before_creating_the_image() {
        let dir = TempDir::new();
        let elf = fixture_elf(format7::MAX_FILE_BYTES as usize + 1);
        let (elf_path, manifest_path) = write_inputs(dir.path(), &elf);
        let image = dir.path().join("must-not-exist.raw");

        let error = seed7(&image, LINEAGE, &elf_path, &manifest_path, false).unwrap_err();
        assert!(error.contains("maximum"));
        assert!(!image.exists());
    }
}
