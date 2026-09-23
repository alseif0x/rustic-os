// SPDX-License-Identifier: Apache-2.0
//! Sparse disposable-disk tests for the first v7 mount/provision owner.
#[path = "volume7/admission.rs"]
mod admission;
#[path = "volume7/failures.rs"]
mod failures;
#[path = "volume7/namespace.rs"]
mod namespace;
#[path = "volume7/read.rs"]
mod read;
mod support;
#[path = "volume7/tracked.rs"]
mod tracked;

use rustic_fs::format7::{
    self, Header7, MAP_WORDS, MAX_EXTENTS, NAME_BYTES, NODES, Node7, RECEIPT_BLOCK_BYTES,
    RECORD_BYTES, RETAINED, Record7, RecordState,
};
use rustic_fs::{DATA_SECTORS, Disk, Error, Extent, Kind, Volume7};
use support::Sparse;

const LINEAGE: [u8; 16] = [0x5a; 16];

fn name_field(name: &[u8]) -> [u8; NAME_BYTES] {
    let mut field = [0; NAME_BYTES];
    field[..name.len()].copy_from_slice(name);
    field
}

fn roots() -> [Node7; NODES] {
    let mut nodes = [Node7::EMPTY; NODES];
    for (index, name) in [b"system".as_slice(), b"data", b"config", b"workspaces"]
        .into_iter()
        .enumerate()
    {
        nodes[index] = Node7 {
            id: index as u32 + 1,
            parent: 0,
            version: 1,
            length: 0,
            kind: Kind::Directory,
            space: index as u8 + 1,
            extents_used: 0,
            extents: [Extent::new(0, 0); MAX_EXTENTS],
            name_length: name.len() as u8,
            name: name_field(name),
            payload_crc32: 0,
        };
    }
    nodes
}

fn file_node(id: u32, version: u64, start: u64, bytes: &[u8]) -> Node7 {
    let mut extents = [Extent::new(0, 0); MAX_EXTENTS];
    extents[0] = Extent::new(start, (bytes.len() as u64).div_ceil(512));
    Node7 {
        id,
        parent: 1,
        version,
        length: bytes.len() as u32,
        kind: Kind::File,
        space: 1,
        extents_used: 1,
        extents,
        name_length: 4,
        name: name_field(b"live"),
        payload_crc32: format7::aggregate(bytes),
    }
}

fn committed_snapshot(start: u64, bytes: &[u8]) -> Record7 {
    let mut extents = [Extent::new(0, 0); MAX_EXTENTS];
    extents[0] = Extent::new(start, 1);
    Record7 {
        subject: 9,
        workspace: 4,
        object: 5,
        instance: 1,
        retry_epoch: 1,
        retry_key: 11,
        previous: 1,
        committed: 2,
        admission_number: 0,
        terminal: 2,
        length: bytes.len() as u32,
        payload_crc32: format7::aggregate(bytes),
        state: RecordState::DirectCommitted,
        prevention: None,
        extents_used: 1,
        extents,
    }
}

fn allocate(map: &mut [u64; MAP_WORDS], sector: u64) {
    map[sector as usize / 64] |= 1u64 << (sector % 64);
}

fn allocate_run(map: &mut [u64; MAP_WORDS], start: u64, sectors: u64) {
    for sector in start..start + sectors {
        allocate(map, sector);
    }
}

fn write_payload(disk: &mut Sparse, relative_sector: u64, data: &[u8]) {
    let mut block = [0u8; 512];
    block[..data.len()].copy_from_slice(data);
    disk.write(format7::PAYLOAD_SECTOR + relative_sector, &block)
        .unwrap();
}

fn write_stream(disk: &mut Sparse, start: u64, bytes: &[u8]) {
    let (blocks, remainder) = bytes.as_chunks::<512>();
    assert!(remainder.is_empty());
    for (index, block) in blocks.iter().enumerate() {
        disk.write(start + index as u64, block).unwrap();
    }
}

fn persist_generation(
    disk: &mut Sparse,
    header: Header7,
    nodes: &[Node7; NODES],
    records: &[Option<Record7>; RETAINED],
    map: &[u64; MAP_WORDS],
) -> Header7 {
    let mut node_bytes = Vec::with_capacity(NODES * format7::NODE_BYTES);
    for node in nodes {
        node_bytes.extend_from_slice(&node.encode().unwrap());
    }
    let mut map_bytes = Vec::with_capacity(format7::MAP_BYTES as usize);
    for word in map {
        map_bytes.extend_from_slice(&word.to_le_bytes());
    }
    let mut receipt_bytes = vec![0; RECEIPT_BLOCK_BYTES];
    for (index, record) in records.iter().enumerate() {
        if let Some(record) = record {
            let at = index * RECORD_BYTES;
            receipt_bytes[at..at + RECORD_BYTES].copy_from_slice(&record.encode().unwrap());
        }
    }

    let nodes_base = format7::nodes_sector(header.generation);
    let map_base = format7::map_sector(header.generation);
    let receipts_base = format7::receipts_sector(header.generation);
    write_stream(disk, nodes_base, &node_bytes);
    write_stream(disk, map_base, &map_bytes);
    write_stream(disk, receipts_base, &receipt_bytes);

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
    header
}

fn empty_generation(
    generation: u8,
    sequence: u64,
) -> (
    [Node7; NODES],
    [Option<Record7>; RETAINED],
    [u64; MAP_WORDS],
    Header7,
) {
    (
        roots(),
        [None; RETAINED],
        [0; MAP_WORDS],
        Header7 {
            generation,
            sequence,
            ..Header7::initial(LINEAGE)
        },
    )
}

fn write_valid_pair(disk: &mut Sparse) {
    let (nodes, records, map, older) = empty_generation(1, 2);
    persist_generation(disk, older, &nodes, &records, &map);
    let (nodes, records, map, newer) = empty_generation(0, 3);
    persist_generation(disk, newer, &nodes, &records, &map);
    disk.flush().unwrap();
}

fn rewrite_header(disk: &mut Sparse, slot: u8, update: impl FnOnce(Header7) -> Header7) {
    let bytes = *disk.live.get(&format7::header_sector(slot)).unwrap();
    let header = Header7::decode(&bytes).unwrap();
    disk.write(
        format7::header_sector(slot),
        &update(header).encode().unwrap(),
    )
    .unwrap();
}

#[test]
fn unformatted_disk_is_refused_and_remains_fenced() {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    assert_eq!(volume.mount_into(&mut disk), Err(Error::Corrupt));
    assert_eq!(volume.header(), Err(Error::Uncertain));
    assert_eq!(volume.node(1), Err(Error::Uncertain));
}

#[test]
fn provision_and_remount_restore_exact_roots_and_free_map() {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    assert_eq!(volume.header().unwrap().sequence, 1);
    assert_eq!(volume.header().unwrap().generation, 0);
    assert_eq!(volume.free_sectors(), Ok(DATA_SECTORS));

    let mut mounted = Volume7::EMPTY;
    mounted.mount_into(&mut disk).unwrap();
    assert_eq!(mounted.header().unwrap().lineage, LINEAGE);
    assert_eq!(mounted.header().unwrap().next, 5);
    assert_eq!(mounted.free_sectors(), Ok(DATA_SECTORS));
    assert!(
        mounted
            .allocation_map()
            .unwrap()
            .iter()
            .all(|word| *word == 0)
    );
    assert_eq!(mounted.recovered_from_header(), Ok(false));

    for (index, name) in ["system", "data", "config", "workspaces"]
        .iter()
        .enumerate()
    {
        let node = mounted.node(index as u32 + 1).unwrap().unwrap();
        assert_eq!(node.kind, Kind::Directory);
        assert_eq!(node.parent, 0);
        assert_eq!(node.space, index as u8 + 1);
        assert_eq!(node.name(), name.as_bytes());
        assert_eq!(node.version, 1);
    }
    assert!(
        mounted
            .retained_records()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn corruption_in_named_nodes_map_or_receipts_is_refused() {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();

    for base in [
        format7::nodes_sector(0),
        format7::map_sector(0),
        format7::receipts_sector(0),
    ] {
        let mut damaged = disk.recover();
        damaged.corrupt(base, 7);
        let mut mounted = Volume7::EMPTY;
        assert_eq!(mounted.mount_into(&mut damaged), Err(Error::Corrupt));
        assert_eq!(mounted.free_sectors(), Err(Error::Uncertain));
    }
}

#[test]
fn live_payload_crc_is_verified_on_mount() {
    let mut disk = Sparse::default();
    let mut nodes = roots();
    let live = [0x31; 512];
    nodes[4] = file_node(5, 2, 10, &live);
    let records = [None; RETAINED];
    let mut map = [0; MAP_WORDS];
    allocate(&mut map, 10);
    write_payload(&mut disk, 10, &live);
    let header = Header7 {
        next: 6,
        sequence: 2,
        ..Header7::initial(LINEAGE)
    };
    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();

    let mut mounted = Volume7::EMPTY;
    mounted.mount_into(&mut disk).unwrap();
    let mut damaged = disk.recover();
    damaged.corrupt(format7::PAYLOAD_SECTOR + 10, 3);
    let mut rejected = Volume7::EMPTY;
    assert_eq!(rejected.mount_into(&mut damaged), Err(Error::Corrupt));
}

#[test]
fn payload_crc_covers_only_logical_bytes_across_noncontiguous_extents() {
    let mut disk = Sparse::default();
    let mut nodes = roots();
    let live: Vec<u8> = (0..513).map(|index| index as u8).collect();
    let mut extents = [Extent::new(0, 0); MAX_EXTENTS];
    extents[0] = Extent::new(10, 1);
    extents[1] = Extent::new(20, 1);
    nodes[4] = Node7 {
        id: 5,
        parent: 1,
        version: 2,
        length: live.len() as u32,
        kind: Kind::File,
        space: 1,
        extents_used: 2,
        extents,
        name_length: 4,
        name: name_field(b"live"),
        payload_crc32: format7::aggregate(&live),
    };
    let records = [None; RETAINED];
    let mut map = [0; MAP_WORDS];
    allocate(&mut map, 10);
    allocate(&mut map, 20);
    let first_sector: &[u8; 512] = live[..512].try_into().unwrap();
    disk.write(format7::PAYLOAD_SECTOR + 10, first_sector)
        .unwrap();
    let mut final_sector = [0xa5; 512];
    final_sector[0] = live[512];
    disk.write(format7::PAYLOAD_SECTOR + 20, &final_sector)
        .unwrap();
    let header = Header7 {
        next: 6,
        sequence: 2,
        ..Header7::initial(LINEAGE)
    };
    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();

    let mut mounted = Volume7::EMPTY;
    mounted.mount_into(&mut disk).unwrap();

    let mut trailing_damage = disk.recover();
    trailing_damage.corrupt(format7::PAYLOAD_SECTOR + 20, 1);
    let mut still_valid = Volume7::EMPTY;
    still_valid.mount_into(&mut trailing_damage).unwrap();

    let mut damaged = disk.recover();
    damaged.corrupt(format7::PAYLOAD_SECTOR + 20, 0);
    let mut rejected = Volume7::EMPTY;
    assert_eq!(rejected.mount_into(&mut damaged), Err(Error::Corrupt));
}

#[test]
fn maximum_size_payload_mounts_across_all_noncontiguous_extents() {
    let mut disk = Sparse::default();
    let mut nodes = roots();
    let payload: Vec<u8> = (0..format7::MAX_FILE_BYTES)
        .map(|index| (index % 251) as u8)
        .collect();
    let mut extents = [Extent::new(0, 0); MAX_EXTENTS];
    let mut map = [0; MAP_WORDS];
    let sectors_per_extent = format7::MAX_FILE_BYTES as u64 / 512 / MAX_EXTENTS as u64;

    for (run, extent) in extents.iter_mut().enumerate() {
        let start = 32 + run as u64 * (sectors_per_extent + 1);
        *extent = Extent::new(start, sectors_per_extent);
        allocate_run(&mut map, start, sectors_per_extent);
        let first = run * sectors_per_extent as usize * 512;
        let last = first + sectors_per_extent as usize * 512;
        let (blocks, remainder) = payload[first..last].as_chunks::<512>();
        assert!(remainder.is_empty());
        for (offset, block) in blocks.iter().enumerate() {
            disk.write(format7::PAYLOAD_SECTOR + start + offset as u64, block)
                .unwrap();
        }
    }

    nodes[4] = Node7 {
        id: 5,
        parent: 1,
        version: 2,
        length: payload.len() as u32,
        kind: Kind::File,
        space: 1,
        extents_used: MAX_EXTENTS as u8,
        extents,
        name_length: 4,
        name: name_field(b"live"),
        payload_crc32: format7::aggregate(&payload),
    };
    let records = [None; RETAINED];
    let header = Header7 {
        next: 6,
        sequence: 2,
        ..Header7::initial(LINEAGE)
    };
    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();

    let mut mounted = Volume7::EMPTY;
    mounted.mount_into(&mut disk).unwrap();
    assert_eq!(
        mounted.node(5).unwrap().unwrap().length,
        payload.len() as u32
    );

    let mut damaged = disk.recover();
    let last_extent = extents[MAX_EXTENTS - 1];
    damaged.corrupt(
        format7::PAYLOAD_SECTOR + last_extent.start + last_extent.sectors - 1,
        511,
    );
    let mut rejected = Volume7::EMPTY;
    assert_eq!(rejected.mount_into(&mut damaged), Err(Error::Corrupt));
}

#[test]
fn retained_candidate_payload_is_checked_even_when_disjoint_from_live_file() {
    let mut disk = Sparse::default();
    let mut nodes = roots();
    let live = [0x31; 512];
    let candidate = [0x72; 512];
    nodes[4] = file_node(5, 3, 10, &live);
    let mut records = [None; RETAINED];
    records[0] = Some(committed_snapshot(20, &candidate));
    let mut map = [0; MAP_WORDS];
    allocate(&mut map, 10);
    allocate(&mut map, 20);
    write_payload(&mut disk, 10, &live);
    write_payload(&mut disk, 20, &candidate);
    let header = Header7 {
        next: 6,
        sequence: 3,
        ..Header7::initial(LINEAGE)
    };
    persist_generation(&mut disk, header, &nodes, &records, &map);
    disk.flush().unwrap();

    let mut mounted = Volume7::EMPTY;
    mounted.mount_into(&mut disk).unwrap();
    assert_eq!(mounted.node(5).unwrap().unwrap().runs()[0].start, 10);
    assert_eq!(
        mounted.retained_records().unwrap()[0]
            .as_ref()
            .unwrap()
            .runs()[0]
            .start,
        20
    );

    let mut damaged = disk.recover();
    damaged.corrupt(format7::PAYLOAD_SECTOR + 20, 9);
    let mut rejected = Volume7::EMPTY;
    assert_eq!(rejected.mount_into(&mut damaged), Err(Error::Corrupt));
}

#[test]
fn a_torn_nonzero_newest_header_recovers_the_previous_generation() {
    let mut disk = Sparse::default();
    write_valid_pair(&mut disk);
    disk.corrupt(format7::header_sector(0), 508);

    let mut mounted = Volume7::EMPTY;
    mounted.mount_into(&mut disk).unwrap();
    assert_eq!(mounted.header().unwrap().sequence, 2);
    assert_eq!(mounted.header().unwrap().generation, 1);
    assert_eq!(mounted.recovered_from_header(), Ok(true));
}

#[test]
fn a_torn_all_zero_newest_header_is_also_reported_as_recovery() {
    let mut disk = Sparse::default();
    write_valid_pair(&mut disk);
    disk.live.insert(format7::header_sector(0), [0; 512]);
    disk.flush().unwrap();

    let mut mounted = Volume7::EMPTY;
    mounted.mount_into(&mut disk).unwrap();
    assert_eq!(mounted.header().unwrap().sequence, 2);
    assert_eq!(mounted.header().unwrap().generation, 1);
    assert_eq!(mounted.recovered_from_header(), Ok(true));
}

#[test]
fn corrupt_structure_named_by_valid_newest_header_does_not_roll_back() {
    let mut disk = Sparse::default();
    write_valid_pair(&mut disk);

    let mut nodes = roots();
    nodes[3] = Node7::EMPTY;
    let records = [None; RETAINED];
    let map = [0; MAP_WORDS];
    let newest = Header7 {
        generation: 0,
        sequence: 3,
        ..Header7::initial(LINEAGE)
    };
    persist_generation(&mut disk, newest, &nodes, &records, &map);
    disk.flush().unwrap();

    let mut mounted = Volume7::EMPTY;
    assert_eq!(mounted.mount_into(&mut disk), Err(Error::Corrupt));
    assert_eq!(mounted.header(), Err(Error::Uncertain));
}

#[test]
fn a_header_in_the_wrong_physical_slot_is_not_a_candidate() {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    let header = *disk.live.get(&format7::header_sector(0)).unwrap();
    disk.write(format7::header_sector(0), &[0; 512]).unwrap();
    disk.write(format7::header_sector(1), &header).unwrap();
    disk.flush().unwrap();

    let mut mounted = Volume7::EMPTY;
    assert_eq!(mounted.mount_into(&mut disk), Err(Error::Corrupt));
}

#[test]
fn impossible_checksum_valid_header_histories_are_refused() {
    for case in 0..5 {
        let mut disk = Sparse::default();
        write_valid_pair(&mut disk);
        match case {
            0 => rewrite_header(&mut disk, 0, |mut header| {
                header.lineage[0] ^= 1;
                header
            }),
            1 => rewrite_header(&mut disk, 0, |mut header| {
                header.sequence = 2;
                header
            }),
            2 => rewrite_header(&mut disk, 0, |mut header| {
                header.sequence = 4;
                header
            }),
            3 => rewrite_header(&mut disk, 1, |mut header| {
                header.epoch = 2;
                header
            }),
            4 => rewrite_header(&mut disk, 1, |mut header| {
                header.next = 6;
                header
            }),
            _ => unreachable!(),
        }
        disk.flush().unwrap();

        let mut mounted = Volume7::EMPTY;
        assert_eq!(
            mounted.mount_into(&mut disk),
            Err(Error::Corrupt),
            "checksum-valid impossible header relation {case}"
        );
    }
}

#[test]
fn selected_newest_generation_ignores_superseded_metadata() {
    let mut disk = Sparse::default();
    write_valid_pair(&mut disk);
    disk.corrupt(format7::nodes_sector(1), 7);

    let mut mounted = Volume7::EMPTY;
    mounted.mount_into(&mut disk).unwrap();
    assert_eq!(mounted.header().unwrap().sequence, 3);
    assert_eq!(mounted.header().unwrap().generation, 0);
    assert_eq!(mounted.recovered_from_header(), Ok(false));
}

#[test]
fn locally_valid_node_change_still_fails_raw_region_checksum() {
    let mut disk = Sparse::default();
    let mut nodes = roots();
    let live = [0x31; 512];
    nodes[4] = file_node(5, 2, 10, &live);
    let records = [None; RETAINED];
    let mut map = [0; MAP_WORDS];
    allocate(&mut map, 10);
    write_payload(&mut disk, 10, &live);
    let header = Header7 {
        next: 6,
        sequence: 2,
        ..Header7::initial(LINEAGE)
    };
    persist_generation(&mut disk, header, &nodes, &records, &map);

    let sector = disk.live.get_mut(&(format7::nodes_sector(0) + 1)).unwrap();
    sector[120..124].copy_from_slice(&0x1234_5678u32.to_le_bytes());
    let checksum = format7::aggregate(&sector[..124]);
    sector[124..128].copy_from_slice(&checksum.to_le_bytes());
    disk.flush().unwrap();

    let mut mounted = Volume7::EMPTY;
    assert_eq!(mounted.mount_into(&mut disk), Err(Error::Corrupt));
}
