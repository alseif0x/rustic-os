// SPDX-License-Identifier: Apache-2.0
//! Bounded v7 payload reads, including fragmented sector mapping.

use super::*;
use rustic_fs::WriteIdentity7;

struct CountingDisk<'a> {
    inner: &'a mut Sparse,
    reads: Vec<u64>,
    writes: usize,
    flushes: usize,
}

impl<'a> CountingDisk<'a> {
    fn new(inner: &'a mut Sparse) -> Self {
        Self {
            inner,
            reads: Vec::new(),
            writes: 0,
            flushes: 0,
        }
    }

    fn reset_counts(&mut self) {
        self.reads.clear();
        self.writes = 0;
        self.flushes = 0;
    }
}

impl Disk for CountingDisk<'_> {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        self.reads.push(sector);
        self.inner.read(sector, bytes)
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        self.writes += 1;
        self.inner.write(sector, bytes)
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.flushes += 1;
        self.inner.flush()
    }
}

fn seed_fragmented_file(disk: &mut Sparse, bytes: &[u8]) {
    assert!(bytes.len() > 1024 && bytes.len() <= 3 * 512);
    let (mut nodes, records, mut map, mut header) = empty_generation(0, 7);
    header.next = 6;
    let first = Extent::new(10, 1);
    let second = Extent::new(40, 2);
    let mut node = file_node(5, 7, first.start, bytes);
    node.extents_used = 2;
    node.extents = [Extent::new(0, 0); MAX_EXTENTS];
    node.extents[0] = first;
    node.extents[1] = second;
    nodes[4] = node;
    allocate_run(&mut map, first.start, first.sectors);
    allocate_run(&mut map, second.start, second.sectors);

    let mut consumed = 0usize;
    for run in [first, second] {
        for sector in 0..run.sectors {
            let mut block = [0u8; 512];
            let take = (bytes.len() - consumed).min(block.len());
            block[..take].copy_from_slice(&bytes[consumed..consumed + take]);
            consumed += take;
            disk.write(format7::PAYLOAD_SECTOR + run.start + sector, &block)
                .unwrap();
        }
    }
    assert_eq!(consumed, bytes.len());
    persist_generation(disk, header, &nodes, &records, &map);
    disk.flush().unwrap();
}

#[test]
fn fragmented_unaligned_ranges_read_only_the_intersecting_sectors() {
    let mut disk = Sparse::default();
    let bytes: Vec<u8> = (0..1300u32).map(|index| (index % 251) as u8).collect();
    seed_fragmented_file(&mut disk, &bytes);

    let mut volume = Volume7::EMPTY;
    let mut disk = CountingDisk::new(&mut disk);
    volume.mount_into(&mut disk).unwrap();
    disk.reset_counts();

    let mut across_runs = [0u8; 600];
    assert_eq!(
        volume.read_range(&mut disk, 5, Some(7), 500, &mut across_runs),
        Ok(across_runs.len())
    );
    assert_eq!(across_runs, bytes[500..1100]);
    assert_eq!(
        disk.reads.as_slice(),
        &[
            format7::PAYLOAD_SECTOR + 10,
            format7::PAYLOAD_SECTOR + 40,
            format7::PAYLOAD_SECTOR + 41,
        ]
    );
    assert_eq!((disk.writes, disk.flushes), (0, 0));

    disk.reset_counts();
    let mut short = [0u8; 40];
    assert_eq!(
        volume.read_range(&mut disk, 5, None, 900, &mut short),
        Ok(short.len())
    );
    assert_eq!(short, bytes[900..940]);
    assert_eq!(disk.reads.as_slice(), &[format7::PAYLOAD_SECTOR + 40]);
    assert_eq!((disk.writes, disk.flushes), (0, 0));
}

#[test]
fn version_directory_and_eof_checks_do_not_read_payload_sectors() {
    let mut disk = Sparse::default();
    let bytes = vec![0x6d; 1300];
    seed_fragmented_file(&mut disk, &bytes);

    let mut volume = Volume7::EMPTY;
    let mut disk = CountingDisk::new(&mut disk);
    volume.mount_into(&mut disk).unwrap();
    disk.reset_counts();

    let mut out = [0xa5; 8];
    assert_eq!(
        volume.read_range(&mut disk, 5, Some(6), 0, &mut out),
        Err(Error::Version)
    );
    assert_eq!(out, [0xa5; 8]);
    assert_eq!(
        volume.read_range(&mut disk, 1, None, 0, &mut out),
        Err(Error::IsDirectory)
    );
    assert_eq!(
        volume.read_range(&mut disk, 5, Some(7), 1301, &mut out),
        Err(Error::Size)
    );
    assert_eq!(
        volume.read_range(&mut disk, 5, Some(7), 1300, &mut out),
        Ok(0)
    );
    assert!(disk.reads.is_empty());
    assert_eq!((disk.writes, disk.flushes), (0, 0));
}

#[test]
fn empty_file_range_returns_zero_without_payload_io() {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    let file = volume.create(&mut disk, 4, b"empty", Kind::File).unwrap();
    let mut disk = CountingDisk::new(&mut disk);

    let mut out = [0x3c; 32];
    assert_eq!(
        volume.read_range(&mut disk, file.id, Some(file.version), 0, &mut out),
        Ok(0)
    );
    assert_eq!(out, [0x3c; 32]);
    assert!(disk.reads.is_empty());
    assert_eq!((disk.writes, disk.flushes), (0, 0));
}

#[test]
fn oversized_range_stops_at_eof_and_leaves_the_tail_untouched() {
    let mut sparse = Sparse::default();
    let bytes = vec![0x7b; 1300];
    seed_fragmented_file(&mut sparse, &bytes);

    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut sparse).unwrap();
    let mut disk = CountingDisk::new(&mut sparse);
    let mut out = [0xa5; 200];

    assert_eq!(
        volume.read_range(&mut disk, 5, Some(7), 1200, &mut out),
        Ok(100)
    );
    assert_eq!(&out[..100], &bytes[1200..]);
    assert_eq!(&out[100..], &[0xa5; 100]);
    assert_eq!(disk.reads, [format7::PAYLOAD_SECTOR + 41]);
    assert_eq!((disk.writes, disk.flushes), (0, 0));
}

#[test]
fn failed_multisector_read_exposes_only_the_prefix_already_read() {
    struct FailAfterOneRead<'a> {
        inner: &'a mut Sparse,
        reads: usize,
    }

    impl Disk for FailAfterOneRead<'_> {
        fn read(&mut self, sector: u64, out: &mut [u8; 512]) -> Result<(), Error> {
            if self.reads == 1 {
                self.reads += 1;
                return Err(Error::Io);
            }
            self.reads += 1;
            self.inner.read(sector, out)
        }

        fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
            self.inner.write(sector, bytes)
        }

        fn flush(&mut self) -> Result<(), Error> {
            self.inner.flush()
        }
    }

    let mut sparse = Sparse::default();
    let bytes: Vec<u8> = (0..1300u32).map(|index| (index % 251) as u8).collect();
    seed_fragmented_file(&mut sparse, &bytes);

    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut sparse).unwrap();
    let mut disk = FailAfterOneRead {
        inner: &mut sparse,
        reads: 0,
    };
    let mut out = [0xa5; 600];

    assert_eq!(
        volume.read_range(&mut disk, 5, Some(7), 500, &mut out),
        Err(Error::Io)
    );
    assert_eq!(&out[..12], &bytes[500..512]);
    assert_eq!(&out[12..], &[0xa5; 588]);
    assert_eq!(disk.reads, 2);
}

#[test]
fn fenced_owner_refuses_range_reads_without_disk_io() {
    let volume = Volume7::EMPTY;
    let mut disk = Sparse::default();
    let mut out = [0; 8];

    assert_eq!(
        volume.read_range(&mut disk, 5, None, 0, &mut out),
        Err(Error::Uncertain)
    );
    assert_eq!(disk.operations, 0);
}

#[test]
fn replaced_file_reads_only_at_the_new_version() {
    let mut sparse = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut sparse, LINEAGE).unwrap();
    let file = volume.create(&mut sparse, 4, b"live", Kind::File).unwrap();
    let identity = WriteIdentity7 {
        subject: 9,
        workspace: 4,
        object: file.id,
        instance: 1,
        retry_epoch: 1,
        retry_key: 11,
    };
    let data = vec![0x3d; 1300];
    let receipt = volume
        .replace_tracked(&mut sparse, identity, file.version, &data)
        .unwrap();
    let mut disk = CountingDisk::new(&mut sparse);
    let mut out = [0; 40];

    assert_eq!(
        volume.read_range(&mut disk, file.id, Some(file.version), 0, &mut out),
        Err(Error::Version)
    );
    assert!(disk.reads.is_empty());
    assert_eq!(
        volume.read_range(&mut disk, file.id, Some(receipt.committed), 1260, &mut out),
        Ok(40)
    );
    assert_eq!(out, [0x3d; 40]);
    assert_eq!(disk.reads.len(), 1);
    assert_eq!((disk.writes, disk.flushes), (0, 0));
}

fn identity(object: u32, key: u64) -> WriteIdentity7 {
    WriteIdentity7 {
        subject: 9,
        workspace: 4,
        object,
        instance: 1,
        retry_epoch: 1,
        retry_key: key,
    }
}

/// Stream a retained snapshot through one sector-sized buffer, as a cold
/// receipt lookup does.
fn stream_retained(volume: &Volume7, disk: &mut impl Disk, record: &Record7) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut block = [0u8; 512];
    loop {
        let read = volume
            .read_retained_range(disk, record, bytes.len() as u64, &mut block)
            .unwrap();
        if read == 0 {
            return bytes;
        }
        bytes.extend_from_slice(&block[..read]);
    }
}

#[test]
fn a_superseded_retained_snapshot_streams_its_own_bytes_after_remount() {
    let mut sparse = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut sparse, LINEAGE).unwrap();
    let file = volume.create(&mut sparse, 4, b"live", Kind::File).unwrap();
    let first: Vec<u8> = (0..1300u32).map(|index| (index % 253) as u8).collect();
    let second = vec![0x5e; 700];
    let older = volume
        .replace_tracked(&mut sparse, identity(file.id, 11), file.version, &first)
        .unwrap();
    let newer = volume
        .replace_tracked(&mut sparse, identity(file.id, 12), older.committed, &second)
        .unwrap();

    let mut mounted = Volume7::EMPTY;
    mounted.mount_into(&mut sparse).unwrap();
    let mut disk = CountingDisk::new(&mut sparse);
    assert_eq!(stream_retained(&mounted, &mut disk, &older), first);
    // Three sectors for 1,300 bytes, and the terminating EOF read costs none.
    assert_eq!(disk.reads.len(), 3);
    assert_eq!((disk.writes, disk.flushes), (0, 0));
    assert_eq!(stream_retained(&mounted, &mut disk, &newer), second);

    // An unaligned range touches only the sectors it intersects.
    disk.reset_counts();
    let mut out = [0u8; 40];
    assert_eq!(
        mounted.read_retained_range(&mut disk, &older, 1100, &mut out),
        Ok(40)
    );
    assert_eq!(out, first[1100..1140]);
    assert_eq!(disk.reads.len(), 1);
    // The live file is the newer version; its range read is unaffected.
    assert_eq!(
        mounted.read_range(&mut disk, file.id, Some(older.committed), 0, &mut out),
        Err(Error::Version)
    );
}

#[test]
fn a_removed_file_keeps_a_readable_retained_snapshot() {
    let mut sparse = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut sparse, LINEAGE).unwrap();
    let file = volume.create(&mut sparse, 4, b"gone", Kind::File).unwrap();
    let data = vec![0x42; 513];
    let record = volume
        .replace_tracked(&mut sparse, identity(file.id, 21), file.version, &data)
        .unwrap();
    volume.remove(&mut sparse, file.id).unwrap();

    let mut mounted = Volume7::EMPTY;
    mounted.mount_into(&mut sparse).unwrap();
    assert_eq!(mounted.stat(file.id), Err(Error::NotFound));
    assert_eq!(stream_retained(&mounted, &mut sparse, &record), data);
}

#[test]
fn only_an_exactly_retained_record_is_read_and_bounds_are_checked_before_io() {
    let mut sparse = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut sparse, LINEAGE).unwrap();
    let file = volume.create(&mut sparse, 4, b"live", Kind::File).unwrap();
    let data = vec![0x17; 1024];
    let record = volume
        .replace_tracked(&mut sparse, identity(file.id, 31), file.version, &data)
        .unwrap();
    let mut disk = CountingDisk::new(&mut sparse);
    let mut out = [0xa5; 16];

    // Any field that differs from the retained record, including a snapshot
    // run pointing elsewhere, names no record this owner vouches for.
    let mut other_key = record;
    other_key.retry_key = 32;
    let mut other_run = record;
    other_run.extents[0] = Extent::new(record.extents[0].start + 100, record.extents[0].sectors);
    let mut longer = record;
    longer.length += 1;
    for forged in [other_key, other_run, longer] {
        assert_eq!(
            volume.read_retained_range(&mut disk, &forged, 0, &mut out),
            Err(Error::NotFound)
        );
    }
    assert_eq!(
        volume.read_retained_range(&mut disk, &record, 1025, &mut out),
        Err(Error::Size)
    );
    assert_eq!(
        volume.read_retained_range(&mut disk, &record, 1024, &mut out),
        Ok(0)
    );
    assert_eq!(out, [0xa5; 16]);
    assert!(disk.reads.is_empty());
    assert_eq!((disk.writes, disk.flushes), (0, 0));

    let fenced = Volume7::EMPTY;
    assert_eq!(
        fenced.read_retained_range(&mut disk, &record, 0, &mut out),
        Err(Error::Uncertain)
    );
    assert!(disk.reads.is_empty());
}

#[test]
fn an_empty_retained_snapshot_reads_zero_bytes_without_io() {
    let mut sparse = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut sparse, LINEAGE).unwrap();
    let file = volume.create(&mut sparse, 4, b"live", Kind::File).unwrap();
    let seeded = volume
        .replace_tracked(&mut sparse, identity(file.id, 41), file.version, &[1, 2, 3])
        .unwrap();
    let empty = volume
        .replace_tracked(&mut sparse, identity(file.id, 42), seeded.committed, &[])
        .unwrap();
    let mut disk = CountingDisk::new(&mut sparse);
    let mut out = [0xa5; 8];
    assert_eq!(
        volume.read_retained_range(&mut disk, &empty, 0, &mut out),
        Ok(0)
    );
    assert_eq!(
        volume.read_retained_range(&mut disk, &empty, 1, &mut out),
        Err(Error::Size)
    );
    assert!(disk.reads.is_empty());
}
