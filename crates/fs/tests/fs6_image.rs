// SPDX-License-Identifier: Apache-2.0
//! File-backed volume images for the v6 layer (#51).
//!
//! The sparse in-memory disks prove the logic; this test proves the same code
//! against a real file with the real volume size, and exports the images it
//! built so `tools/fs6_test.py` can verify them with an independent reader.
//! Only the prefix a volume actually uses is exported, so the evidence stays
//! small while the payload still lives at its true sector offsets.
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use rustic_fs::{
    DATA_SECTORS, Disk, Error, Kind, Node6, PAYLOAD_SECTOR, Retry, VOLUME_SECTORS, Volume, Volume6,
    mount6, provision6, upgrade6,
};

const LINEAGE: [u8; 16] = [3; 16];
const V5_SECTORS: u64 = 174;
/// 200,000 bytes is far beyond a v5 file, so the payload must span extents.
const BIG: usize = 200_000;

/// A real file addressed by sector, sized once and read as zeros in its holes.
struct FileDisk {
    file: File,
}

impl FileDisk {
    fn create(path: &Path, sectors: u64) -> Self {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .expect("create image");
        file.set_len(sectors * 512).expect("size image");
        Self { file }
    }
    fn reopen(path: &Path, sectors: u64) -> Self {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .expect("open image");
        file.set_len(sectors * 512).expect("size image");
        Self { file }
    }
    fn export(&mut self, name: &str, sectors: u64) {
        let Some(directory) = std::env::var_os("RUSTIC_FS6_EXPORT") else {
            return;
        };
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).expect("export directory");
        let mut bytes = vec![0; (sectors * 512) as usize];
        self.file.seek(SeekFrom::Start(0)).expect("seek");
        self.file.read_exact(&mut bytes).expect("read image");
        std::fs::write(directory.join(name), bytes).expect("export image");
    }
}

impl Disk for FileDisk {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        self.file
            .seek(SeekFrom::Start(sector * 512))
            .map_err(|_| Error::Io)?;
        match self.file.read_exact(bytes) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                bytes.fill(0);
                Ok(())
            }
            Err(_) => Err(Error::Io),
        }
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        self.file
            .seek(SeekFrom::Start(sector * 512))
            .map_err(|_| Error::Io)?;
        self.file.write_all(bytes).map_err(|_| Error::Io)
    }
    fn flush(&mut self) -> Result<(), Error> {
        self.file.sync_all().map_err(|_| Error::Io)
    }
}

fn image(name: &str) -> PathBuf {
    let unique = format!("rustic-fs6-{}-{}.img", std::process::id(), name);
    std::env::temp_dir().join(unique)
}

fn pattern(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index % 251) as u8).collect()
}

/// The end of the highest payload extent any node holds, so an exported image
/// carries every sector the volume references and nothing beyond them.
fn payload_end(volume: &Volume6) -> u64 {
    volume
        .nodes
        .iter()
        .flat_map(|node| node.runs())
        .map(|run| run.start + run.sectors)
        .max()
        .unwrap_or(0)
}

#[test]
fn a_provisioned_volume_round_trips_through_a_real_image_file() {
    let path = image("provisioned");
    let mut disk = FileDisk::create(&path, VOLUME_SECTORS);
    let mut volume = provision6(&mut disk, LINEAGE).expect("provision");
    let bytes = pattern(BIG);
    let mut record = Node6::EMPTY;
    record.id = 5;
    record.parent = 4;
    record.kind = Kind::File;
    record.version = 1;
    record.name[..8].copy_from_slice(b"artifact");
    record.name_length = 8;
    volume.nodes[4] = record;
    assert_eq!(volume.write_file(&mut disk, 4, 1, &bytes), Ok(2));
    let used = payload_end(&volume);
    disk.export("v6-provisioned.img", PAYLOAD_SECTOR + used);

    // A fresh handle reads what a fresh boot would read.
    let mut reopened = FileDisk::reopen(&path, VOLUME_SECTORS);
    let mounted = mount6(&mut reopened).expect("mount from file");
    let node = mounted.node(5).expect("file node");
    assert_eq!(node.length as usize, BIG);
    assert_eq!(node.version, 2);
    assert!(node.runs().len() >= 2, "the fixture must span extents");
    assert_eq!(mounted.free_sectors(), DATA_SECTORS - used);
    let mut out = vec![0; BIG];
    assert_eq!(mounted.read_file(&mut reopened, node, &mut out), Ok(BIG));
    assert_eq!(out, bytes);
    drop(reopened);
    std::fs::remove_file(&path).ok();
}

#[test]
fn a_v5_image_is_migrated_in_place_and_keeps_its_files() {
    let path = image("migrated");
    let mut disk = FileDisk::create(&path, VOLUME_SECTORS);
    let mut source = Volume::initialize(&mut disk).expect("v5 initialize");
    let dir = source
        .create(&mut disk, 4, b"project", Kind::Directory)
        .expect("directory");
    let notes = source
        .create(&mut disk, dir.id, b"notes.txt", Kind::File)
        .expect("file");
    let notes = source
        .replace(&mut disk, notes.id, notes.version, b"a v5 record")
        .expect("content");
    disk.export("v5-source.img", V5_SECTORS);

    let upgraded = upgrade6(&mut disk, LINEAGE).expect("migrate");
    assert_eq!(upgraded.report.files, 1);
    assert_eq!(upgraded.report.directories, 5);
    let used = payload_end(&upgraded.volume);
    disk.export("v6-migrated.img", PAYLOAD_SECTOR + used);

    drop(disk);
    let mut reopened = FileDisk::reopen(&path, VOLUME_SECTORS);
    let mounted = mount6(&mut reopened).expect("mount the migrated volume");
    let node = mounted.node(notes.id).expect("migrated file");
    assert_eq!(node.version, notes.version);
    assert_eq!(node.parent, dir.id);
    assert_eq!(node.name(), b"notes.txt");
    let mut out = vec![0; node.length as usize];
    assert_eq!(
        mounted.read_file(&mut reopened, node, &mut out),
        Ok(out.len())
    );
    assert_eq!(out, b"a v5 record");
    // The v5 layout is gone: what was a v5 volume is not one any more.
    assert_eq!(Volume::mount(&mut reopened).err(), Some(Error::Corrupt));
    drop(reopened);
    std::fs::remove_file(&path).ok();
}

#[test]
fn a_tracked_write_exports_the_receipt_binding() {
    let path = image("tracked");
    let mut disk = FileDisk::create(&path, VOLUME_SECTORS);
    let mut volume = provision6(&mut disk, LINEAGE).expect("provision");
    let bytes = pattern(4096);
    let mut record = Node6::EMPTY;
    record.id = 5;
    record.parent = 4;
    record.kind = Kind::File;
    record.version = 1;
    record.name[..8].copy_from_slice(b"artifact");
    record.name_length = 8;
    volume.nodes[4] = record;
    let receipt = volume
        .write_tracked(
            &mut disk,
            4,
            1,
            Retry {
                lineage: LINEAGE,
                epoch: 1,
                key: 21,
            },
            &bytes,
        )
        .expect("tracked write");
    assert_eq!(
        (
            receipt.id,
            receipt.previous,
            receipt.committed,
            receipt.length
        ),
        (5, 1, 2, 4096)
    );
    let used = payload_end(&volume);
    disk.export("v6-tracked.img", PAYLOAD_SECTOR + used);

    let mut reopened = FileDisk::reopen(&path, VOLUME_SECTORS);
    let mounted = mount6(&mut reopened).expect("mount from file");
    assert_eq!(mounted.find_receipt(receipt.retry), Ok(Some(&receipt)));
    let node = mounted.node(5).expect("file node");
    assert_eq!(node.version, receipt.committed);
    let mut out = vec![0; bytes.len()];
    assert_eq!(
        mounted.read_file(&mut reopened, node, &mut out),
        Ok(bytes.len())
    );
    assert_eq!(out, bytes);
    drop(reopened);
    std::fs::remove_file(&path).ok();
}
