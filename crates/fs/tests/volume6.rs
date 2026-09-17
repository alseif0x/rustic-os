// SPDX-License-Identifier: Apache-2.0
//! Host tests for the v6 volume layer: provision, mount, verified structures and
//! payload I/O through extents (#51).
use std::collections::BTreeMap;

use rustic_fs::{DATA_BYTES_V6, Disk, Error, Kind, MAX_FILE_V6, Node6, mount6, provision6};

/// Sparse in-memory disk: only written sectors are stored, so a 64 MiB payload
/// region costs nothing until it is used. `durable` models flush.
#[derive(Default)]
struct Sparse {
    live: BTreeMap<u64, [u8; 512]>,
    durable: BTreeMap<u64, [u8; 512]>,
}

impl Sparse {
    fn recover(&self) -> Self {
        Self {
            live: self.durable.clone(),
            durable: self.durable.clone(),
        }
    }
    fn corrupt(&mut self, sector: u64, offset: usize) {
        self.live.get_mut(&sector).expect("written sector")[offset] ^= 1;
    }
}

impl Disk for Sparse {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        *bytes = self.live.get(&sector).copied().unwrap_or([0; 512]);
        Ok(())
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        self.live.insert(sector, *bytes);
        Ok(())
    }
    fn flush(&mut self) -> Result<(), Error> {
        self.durable = self.live.clone();
        Ok(())
    }
}

#[test]
fn provision_then_mount_restores_the_roots_and_the_whole_free_region() {
    let mut disk = Sparse::default();
    // An unformatted disk is not mountable: the header is missing, not guessed.
    assert_eq!(mount6(&mut disk).err(), Some(Error::Corrupt));

    let volume = provision6(&mut disk).expect("provision");
    assert_eq!(volume.free_sectors(), rustic_fs::DATA_SECTORS);
    let mounted = mount6(&mut disk).expect("mount");
    for (index, name) in ["system", "data", "config", "workspaces"]
        .iter()
        .enumerate()
    {
        let node = mounted.node(index as u32 + 1).expect("root directory");
        assert_eq!(node.kind, Kind::Directory);
        let len = node.name_length as usize;
        assert_eq!(&node.name[..len], name.as_bytes());
    }
    assert_eq!(mounted.free_sectors(), rustic_fs::DATA_SECTORS);
}

#[test]
fn a_large_file_round_trips_across_extents_and_reclaims_exactly() {
    let mut disk = Sparse::default();
    let mut volume = provision6(&mut disk).expect("provision");
    let initial = volume.free_sectors();

    // The layer owns payload, not identity: the caller creates the record.
    let mut record = Node6::EMPTY;
    record.id = 5;
    record.parent = 4;
    record.kind = Kind::File;
    record.name[..3].copy_from_slice(b"app");
    record.name_length = 3;
    volume.nodes[4] = record;

    // 220,000 bytes: far beyond the v5 record's 16-bit length and one bank.
    let bytes: Vec<u8> = (0..220_000u32).map(|index| (index % 251) as u8).collect();
    volume.write_file(&mut disk, 4, &bytes).expect("write");
    let sectors = (bytes.len() as u64).div_ceil(512);
    assert_eq!(volume.free_sectors(), initial - sectors);

    let mounted = mount6(&mut disk).expect("remount");
    let node = mounted.node(5).expect("file node");
    assert_eq!(node.length as usize, bytes.len());
    assert!(node.extents_used >= 1);
    let mut out = vec![0u8; bytes.len()];
    let mut disk = disk.recover();
    assert_eq!(
        mounted.read_file(&mut disk, node, &mut out),
        Ok(bytes.len())
    );
    assert_eq!(out, bytes);

    // Removing the file returns every sector it held.
    let mut volume = mount6(&mut disk).expect("mount for removal");
    volume.remove_file(&mut disk, 4).expect("remove");
    assert_eq!(volume.free_sectors(), initial);
    assert!(mount6(&mut disk).expect("mount").node(5).is_none());
}

#[test]
fn a_file_larger_than_the_limit_is_refused_before_anything_is_written() {
    let mut disk = Sparse::default();
    let mut volume = provision6(&mut disk).expect("provision");
    let initial = volume.free_sectors();
    let oversized = vec![0u8; MAX_FILE_V6 + 1];
    assert_eq!(
        volume.write_file(&mut disk, 4, &oversized),
        Err(Error::Size)
    );
    assert_eq!(volume.free_sectors(), initial);
    assert!(volume.node(5).is_none());
}

#[test]
fn a_corrupt_or_torn_volume_is_refused_instead_of_mounted() {
    let mut disk = Sparse::default();
    provision6(&mut disk).expect("provision");
    let mut volume = provision6(&mut disk).expect("reprovision");
    volume.write_file(&mut disk, 4, b"payload").expect("write");

    // A single flipped byte in the node table must fail the header's checksum.
    let mut torn = Sparse {
        live: disk.durable.clone(),
        durable: disk.durable.clone(),
    };
    torn.corrupt(9, 0);
    assert_eq!(mount6(&mut torn).err(), Some(Error::Corrupt));

    // So must a flipped byte in the free-space map.
    let mut map_torn = Sparse {
        live: disk.durable.clone(),
        durable: disk.durable.clone(),
    };
    map_torn.corrupt(73, 3);
    assert_eq!(mount6(&mut map_torn).err(), Some(Error::Corrupt));

    // And so must a flipped byte in the header itself.
    let mut header_torn = Sparse {
        live: disk.durable.clone(),
        durable: disk.durable.clone(),
    };
    header_torn.corrupt(8, 12);
    assert_eq!(mount6(&mut header_torn).err(), Some(Error::Corrupt));

    // The untorn volume still mounts, so the refusals above are specific.
    assert!(mount6(&mut disk).is_ok());
    assert_eq!(DATA_BYTES_V6, 64 * 1024 * 1024);
    let _ = Node6::EMPTY;
}
