// SPDX-License-Identifier: Apache-2.0
//! File-backed V7 images for the independent reader (#51).
//!
//! The sparse in-memory disks prove the owner's logic; this test drives the
//! same public `Volume7` owner against real, full-size sparse image files and,
//! when `RUSTIC_FS7_EXPORT` names a directory, leaves them there so
//! `tools/fs7_test.py` can verify them with `terminal_support/oracle7.py`, a
//! reader written from `docs/WORKSPACE-FORMAT7.md` rather than from this crate.
//! The payload patterns are deliberately simple so the Python side can
//! regenerate them independently instead of trusting an exported digest.
use core::task::Poll;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use rustic_fs::format7::{
    self, Header7, MAP_WORDS, MAX_EXTENTS, NAME_BYTES, NODES, Node7, RECEIPT_BLOCK_BYTES,
    RECORD_BYTES, RETAINED, Record7, RecordState,
};
use rustic_fs::{
    Disk, Error, Extent, Kind, PollDisk, PollDisk7, PollPublication7, PreventionReason,
    Publication7Phase, Volume7, WriteIdentity7,
};

const HISTORY_LINEAGE: [u8; 16] = [0x71; 16];
const FRAGMENTED_LINEAGE: [u8; 16] = [0x72; 16];
const MAINTAINED_LINEAGE: [u8; 16] = [0x73; 16];

/// A full-size sparse image file addressed by sector. Reads past the end fail
/// instead of returning zeros, like the host `rustic-volume` disk.
struct FileDisk {
    file: File,
}

impl FileDisk {
    fn create(path: &Path) -> Self {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .expect("create image");
        file.set_len(format7::VOLUME_SECTORS * 512)
            .expect("size image");
        Self { file }
    }
}

impl Disk for FileDisk {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        if sector >= format7::VOLUME_SECTORS {
            return Err(Error::Io);
        }
        self.file
            .seek(SeekFrom::Start(sector * 512))
            .map_err(|_| Error::Io)?;
        self.file.read_exact(bytes).map_err(|_| Error::Io)
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        if sector >= format7::VOLUME_SECTORS {
            return Err(Error::Io);
        }
        self.file
            .seek(SeekFrom::Start(sector * 512))
            .map_err(|_| Error::Io)?;
        self.file.write_all(bytes).map_err(|_| Error::Io)
    }
    fn flush(&mut self) -> Result<(), Error> {
        self.file.sync_all().map_err(|_| Error::Io)
    }
}

/// Every command completes on its first poll; this is a host image, not a device.
impl PollDisk for FileDisk {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        Poll::Ready(self.write(sector, bytes))
    }
    fn poll_flush(&mut self) -> Poll<Result<(), Error>> {
        Poll::Ready(self.flush())
    }
}

impl PollDisk7 for FileDisk {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), Error>> {
        Poll::Ready(self.read(sector, bytes))
    }
}

/// Exported images go to `RUSTIC_FS7_EXPORT`; otherwise to a temporary path
/// that the test removes.
struct Image {
    path: PathBuf,
    keep: bool,
}

impl Image {
    fn new(name: &str) -> Self {
        match std::env::var_os("RUSTIC_FS7_EXPORT") {
            Some(directory) => {
                let directory = PathBuf::from(directory);
                std::fs::create_dir_all(&directory).expect("export directory");
                Self {
                    path: directory.join(name),
                    keep: true,
                }
            }
            None => Self {
                path: std::env::temp_dir()
                    .join(format!("rustic-fs7-{}-{name}", std::process::id())),
                keep: false,
            },
        }
    }
}

impl Drop for Image {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Byte `i` of pattern `seed` is `(i * (2 * seed + 1) + seed) % 251`.
/// `tools/fs7_test.py` regenerates the same bytes on its own.
fn pattern(seed: usize, length: usize) -> Vec<u8> {
    (0..length)
        .map(|index| ((index * (2 * seed + 1) + seed) % 251) as u8)
        .collect()
}

fn identity(workspace: u32, object: u32, instance: u64, epoch: u64, key: u64) -> WriteIdentity7 {
    WriteIdentity7 {
        subject: 1,
        workspace,
        object,
        instance,
        retry_epoch: epoch,
        retry_key: key,
    }
}

fn settle(publication: &mut PollPublication7<'_, FileDisk>) -> Record7 {
    loop {
        match publication.poll_advance() {
            Poll::Ready(Ok(Publication7Phase::Committed)) => {
                return publication.result().expect("committed record");
            }
            Poll::Ready(Ok(Publication7Phase::Failed | Publication7Phase::Uncertain)) => {
                panic!("publication did not commit")
            }
            Poll::Ready(Err(error)) => panic!("publication failed: {error:?}"),
            Poll::Ready(Ok(_)) | Poll::Pending => (),
        }
    }
}

fn read_all(volume: &Volume7, disk: &mut FileDisk, id: u32) -> Vec<u8> {
    let node = volume.stat(id).expect("stat");
    let mut out = vec![0; node.length as usize];
    let count = volume
        .read_range(disk, id, Some(node.version), 0, &mut out)
        .expect("read");
    assert_eq!(count, out.len());
    out
}

fn remount(disk: &mut FileDisk) -> Box<Volume7> {
    let mut volume = Box::new(Volume7::EMPTY);
    volume.mount_into(disk).expect("remount");
    volume
}

/// Tracked replacements, a removal with a retained snapshot, an unresolved
/// admission, a cancelled admission and an executed admission, in one epoch.
/// The final publication is a direct replacement of `delta`, so the older
/// generation still shows `delta` empty at the previous sequence.
#[test]
fn history_image_retains_every_record_state_across_a_remount() {
    let image = Image::new("v7-history.img");
    let mut disk = FileDisk::create(&image.path);
    let mut volume = Box::new(Volume7::EMPTY);
    volume.provision_into(&mut disk, HISTORY_LINEAGE).unwrap();
    let app = volume
        .create(&mut disk, 4, b"app", Kind::Directory)
        .unwrap();
    let [alpha, beta, gamma, delta] = [b"alpha".as_slice(), b"beta", b"gamma", b"delta"]
        .map(|name| volume.create(&mut disk, app.id, name, Kind::File).unwrap());
    let epoch = volume.header().unwrap().epoch;

    let a1 = volume
        .replace_tracked(
            &mut disk,
            identity(app.id, alpha.id, alpha.version, epoch, 1),
            alpha.version,
            &pattern(1, 200_000),
        )
        .unwrap();
    let a2 = volume
        .replace_tracked(
            &mut disk,
            identity(app.id, alpha.id, alpha.version, epoch, 2),
            a1.committed,
            &pattern(2, 4096),
        )
        .unwrap();
    volume
        .replace_tracked(
            &mut disk,
            identity(app.id, beta.id, beta.version, epoch, 3),
            beta.version,
            &pattern(3, 1000),
        )
        .unwrap();
    volume.remove(&mut disk, beta.id).unwrap();

    let gamma_candidate = pattern(4, 3000);
    let admitted = settle(
        &mut volume
            .prepare_admission(
                &mut disk,
                identity(app.id, gamma.id, gamma.version, epoch, 4),
                gamma.version,
                &gamma_candidate,
            )
            .unwrap(),
    );
    assert_eq!(admitted.state, RecordState::Admitted);

    let delta_candidate = pattern(5, 700);
    let delta_identity = identity(app.id, delta.id, delta.version, epoch, 5);
    settle(
        &mut volume
            .prepare_admission(&mut disk, delta_identity, delta.version, &delta_candidate)
            .unwrap(),
    );
    let cancelled = settle(
        &mut volume
            .prepare_cancellation(
                &mut disk,
                delta_identity,
                delta.version,
                PreventionReason::Requested,
            )
            .unwrap(),
    );
    assert_eq!(cancelled.state, RecordState::Cancelled);

    let alpha_candidate = pattern(6, 10_000);
    let alpha_identity = identity(app.id, alpha.id, alpha.version, epoch, 6);
    settle(
        &mut volume
            .prepare_admission(&mut disk, alpha_identity, a2.committed, &alpha_candidate)
            .unwrap(),
    );
    let executed = settle(
        &mut volume
            .prepare_execute(&mut disk, alpha_identity, a2.committed)
            .unwrap(),
    );
    assert_eq!(executed.state, RecordState::AdmittedCommitted);

    let before_last = *volume.header().unwrap();
    volume
        .replace_tracked(
            &mut disk,
            identity(app.id, delta.id, delta.version, epoch, 7),
            delta.version,
            &pattern(7, 5000),
        )
        .unwrap();
    assert_eq!(volume.header().unwrap().sequence, before_last.sequence + 1);

    let volume = remount(&mut disk);
    assert!(!volume.recovered_from_header().unwrap());
    assert_eq!(read_all(&volume, &mut disk, alpha.id), alpha_candidate);
    assert_eq!(read_all(&volume, &mut disk, delta.id), pattern(7, 5000));
    assert!(read_all(&volume, &mut disk, gamma.id).is_empty());
    assert_eq!(volume.stat(beta.id), Err(Error::NotFound));
    let records = volume.retained_records().unwrap();
    assert_eq!(records.iter().flatten().count(), 7);
}

fn name_field(name: &[u8]) -> [u8; NAME_BYTES] {
    let mut field = [0; NAME_BYTES];
    field[..name.len()].copy_from_slice(name);
    field
}

fn node(id: u32, parent: u32, space: u8, version: u64, name: &[u8]) -> Node7 {
    Node7 {
        id,
        parent,
        version,
        length: 0,
        kind: Kind::Directory,
        space,
        extents_used: 0,
        extents: [Extent::new(0, 0); MAX_EXTENTS],
        name_length: name.len() as u8,
        name: name_field(name),
        payload_crc32: 0,
    }
}

fn file(id: u32, version: u64, name: &[u8], runs: &[Extent], bytes: &[u8]) -> Node7 {
    let mut extents = [Extent::new(0, 0); MAX_EXTENTS];
    extents[..runs.len()].copy_from_slice(runs);
    Node7 {
        length: bytes.len() as u32,
        kind: Kind::File,
        extents_used: runs.len() as u8,
        extents,
        payload_crc32: format7::aggregate(bytes),
        ..node(id, 2, 2, version, name)
    }
}

fn write_runs(disk: &mut FileDisk, runs: &[Extent], bytes: &[u8]) {
    let mut chunks = bytes.chunks(512);
    for run in runs {
        for sector in run.start..run.end() {
            let mut block = [0u8; 512];
            if let Some(chunk) = chunks.next() {
                block[..chunk.len()].copy_from_slice(chunk);
            }
            disk.write(format7::PAYLOAD_SECTOR + sector, &block)
                .unwrap();
        }
    }
}

fn write_region(disk: &mut FileDisk, first: u64, bytes: &[u8]) {
    for (index, chunk) in bytes.chunks(512).enumerate() {
        disk.write(first + index as u64, chunk.try_into().unwrap())
            .unwrap();
    }
}

/// Persist one hand-built generation through the public format7 codecs only.
fn persist(
    disk: &mut FileDisk,
    header: Header7,
    nodes: &[Node7; NODES],
    records: &[Option<Record7>; RETAINED],
    map: &[u64; MAP_WORDS],
) {
    let node_bytes: Vec<u8> = nodes.iter().flat_map(|n| n.encode().unwrap()).collect();
    let map_bytes: Vec<u8> = map.iter().flat_map(|word| word.to_le_bytes()).collect();
    let mut receipt_bytes = vec![0; RECEIPT_BLOCK_BYTES];
    for (index, record) in records.iter().enumerate() {
        if let Some(record) = record {
            receipt_bytes[index * RECORD_BYTES..(index + 1) * RECORD_BYTES]
                .copy_from_slice(&record.encode().unwrap());
        }
    }
    write_region(disk, format7::nodes_sector(header.generation), &node_bytes);
    write_region(disk, format7::map_sector(header.generation), &map_bytes);
    write_region(
        disk,
        format7::receipts_sector(header.generation),
        &receipt_bytes,
    );
    let header = Header7 {
        nodes_checksum: format7::aggregate(&node_bytes),
        map_checksum: format7::aggregate(&map_bytes),
        receipts_checksum: format7::aggregate(&receipt_bytes),
        ..header
    };
    disk.write(
        format7::header_sector(header.generation),
        &header.encode().unwrap(),
    )
    .unwrap();
    disk.flush().unwrap();
}

/// A hand-built generation with a three-run file out of disk order, aliased by
/// its committed retained snapshot, which the owner then mounts and extends
/// with one real tracked replacement.
#[test]
fn fragmented_image_mounts_and_publishes_over_noncontiguous_runs() {
    let image = Image::new("v7-fragmented.img");
    let mut disk = FileDisk::create(&image.path);
    let runs = [Extent::new(10, 2), Extent::new(0, 3), Extent::new(20, 1)];
    let frag = pattern(8, 3000);
    let other = pattern(9, 100);
    let mut nodes = [Node7::EMPTY; NODES];
    for (index, name) in [b"system".as_slice(), b"data", b"config", b"workspaces"]
        .into_iter()
        .enumerate()
    {
        nodes[index] = node(index as u32 + 1, 0, index as u8 + 1, 1, name);
    }
    nodes[4] = file(5, 3, b"frag", &runs, &frag);
    nodes[5] = file(6, 2, b"other", &[Extent::new(40, 1)], &other);
    let mut record_runs = [Extent::new(0, 0); MAX_EXTENTS];
    record_runs[..3].copy_from_slice(&runs);
    let mut records = [None; RETAINED];
    records[0] = Some(Record7 {
        subject: 1,
        workspace: 2,
        object: 5,
        instance: 1,
        retry_epoch: 1,
        retry_key: 1,
        previous: 1,
        committed: 3,
        admission_number: 0,
        terminal: 3,
        length: frag.len() as u32,
        payload_crc32: format7::aggregate(&frag),
        state: RecordState::DirectCommitted,
        prevention: None,
        extents_used: 3,
        extents: record_runs,
    });
    let mut map = [0u64; MAP_WORDS];
    for run in runs.iter().chain(&[Extent::new(40, 1)]) {
        for sector in run.start..run.end() {
            map[sector as usize / 64] |= 1 << (sector % 64);
        }
    }
    write_runs(&mut disk, &runs, &frag);
    write_runs(&mut disk, &[Extent::new(40, 1)], &other);
    persist(
        &mut disk,
        Header7 {
            sequence: 3,
            next: 7,
            ..Header7::initial(FRAGMENTED_LINEAGE)
        },
        &nodes,
        &records,
        &map,
    );

    let mut volume = remount(&mut disk);
    assert_eq!(read_all(&volume, &mut disk, 5), frag);
    volume
        .replace_tracked(&mut disk, identity(2, 6, 1, 1, 2), 2, &pattern(10, 2000))
        .unwrap();
    let volume = remount(&mut disk);
    assert_eq!(read_all(&volume, &mut disk, 5), frag);
    assert_eq!(read_all(&volume, &mut disk, 6), pattern(10, 2000));
}

/// Explicit retention maintenance publishes epoch 2, reclaims the terminal
/// snapshot, and the next replacement is retained in the new epoch.
#[test]
fn maintained_image_advances_the_retry_epoch() {
    let image = Image::new("v7-maintained.img");
    let mut disk = FileDisk::create(&image.path);
    let mut volume = Box::new(Volume7::EMPTY);
    volume
        .provision_into(&mut disk, MAINTAINED_LINEAGE)
        .unwrap();
    let dir = volume
        .create(&mut disk, 3, b"settings", Kind::Directory)
        .unwrap();
    let target = volume
        .create(&mut disk, dir.id, b"value", Kind::File)
        .unwrap();
    let first = volume
        .replace_tracked(
            &mut disk,
            identity(dir.id, target.id, target.version, 1, 1),
            target.version,
            &pattern(11, 600),
        )
        .unwrap();
    let second = volume
        .replace_tracked(
            &mut disk,
            identity(dir.id, target.id, target.version, 1, 2),
            first.committed,
            &pattern(12, 1500),
        )
        .unwrap();
    assert_eq!(volume.maintain_retention(&mut disk).unwrap(), 2);
    volume
        .replace_tracked(
            &mut disk,
            identity(dir.id, target.id, target.version, 2, 1),
            second.committed,
            &pattern(13, 800),
        )
        .unwrap();
    let volume = remount(&mut disk);
    assert_eq!(volume.header().unwrap().epoch, 2);
    assert_eq!(read_all(&volume, &mut disk, target.id), pattern(13, 800));
    assert_eq!(
        volume.retained_records().unwrap().iter().flatten().count(),
        1
    );
}

/// Truncation that removes a referenced payload sector fails the mount with an
/// I/O error. Mount reads only the header, the selected generation and the
/// referenced payload, so it does not itself check the medium's capacity: a
/// missing unused tail sector is left to the caller (`report7` requires the
/// exact image size).
#[test]
fn truncated_image_refuses_mount_only_when_a_referenced_sector_is_missing() {
    let image = Image {
        path: std::env::temp_dir().join(format!("rustic-fs7-{}-truncated.img", std::process::id())),
        keep: false,
    };
    let mut disk = FileDisk::create(&image.path);
    let mut volume = Box::new(Volume7::EMPTY);
    volume.provision_into(&mut disk, HISTORY_LINEAGE).unwrap();
    let target = volume.create(&mut disk, 2, b"payload", Kind::File).unwrap();
    let record = volume
        .replace_tracked(
            &mut disk,
            identity(2, target.id, target.version, 1, 1),
            target.version,
            &pattern(14, 4000),
        )
        .unwrap();
    let last = record.runs().iter().map(|run| run.end()).max().unwrap();

    disk.file
        .set_len((format7::VOLUME_SECTORS - 1) * 512)
        .unwrap();
    let mut remounted = Box::new(Volume7::EMPTY);
    remounted.mount_into(&mut disk).unwrap();

    disk.file
        .set_len((format7::PAYLOAD_SECTOR + last - 1) * 512)
        .unwrap();
    assert_eq!(remounted.mount_into(&mut disk), Err(Error::Io));
    assert_eq!(remounted.header(), Err(Error::Uncertain));
}
