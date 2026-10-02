// SPDX-License-Identifier: Apache-2.0
use super::*;
use rustic_fs::{Stage7Kind, WriteIdentity7};

fn seed() -> (Volume7, Sparse, Node7) {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    let node = volume
        .create(&mut disk, 4, b"ordinary", Kind::File)
        .unwrap();
    (volume, disk, node)
}

fn bytes(volume: &Volume7, disk: &mut Sparse, node: Node7) -> Vec<u8> {
    let mut result = vec![0; node.length as usize];
    assert_eq!(
        volume.read_range(disk, node.id, Some(node.version), 0, &mut result),
        Ok(result.len())
    );
    result
}

#[test]
fn repeated_ordinary_writes_and_emptying_do_not_allocate_receipts() {
    let (mut volume, mut disk, mut node) = seed();
    for index in 0..32 {
        let content = vec![index; 513];
        let previous = node.version;
        node = volume
            .replace(&mut disk, node.id, previous, &content)
            .unwrap();
        assert!(node.version > previous);
        assert_eq!(bytes(&volume, &mut disk, node), content);
        assert_eq!(volume.free_sectors(), Ok(DATA_SECTORS - 2));
        assert!(
            volume
                .retained_records()
                .unwrap()
                .iter()
                .all(Option::is_none)
        );
    }
    node = volume
        .replace(&mut disk, node.id, node.version, &[])
        .unwrap();
    assert_eq!(node.length, 0);
    assert_eq!(volume.free_sectors(), Ok(DATA_SECTORS));
    let mut recovered = Volume7::EMPTY;
    recovered.mount_into(&mut disk.recover()).unwrap();
    assert_eq!(recovered.stat(node.id), Ok(node));
}

#[test]
fn a_full_receipt_table_preserves_snapshots_and_allows_ordinary_writes() {
    let (mut volume, mut disk, mut node) = seed();
    let mut records = Vec::new();
    for key in 1..=RETAINED {
        let content = vec![key as u8; 513];
        let identity = WriteIdentity7 {
            subject: 9,
            workspace: 4,
            object: node.id,
            instance: 1,
            retry_epoch: 1,
            retry_key: key as u64,
        };
        let record = volume
            .replace_tracked(&mut disk, identity, node.version, &content)
            .unwrap();
        records.push((record, content));
        node = volume.stat(node.id).unwrap();
    }
    let retained_before = *volume.retained_records().unwrap();
    for content in [b"manual version one".as_slice(), b"manual version two", b""] {
        node = volume
            .replace(&mut disk, node.id, node.version, content)
            .unwrap();
        assert_eq!(bytes(&volume, &mut disk, node), content);
        assert_eq!(*volume.retained_records().unwrap(), retained_before);
    }
    let mut mounted = Volume7::EMPTY;
    let mut recovered = disk.recover();
    mounted.mount_into(&mut recovered).unwrap();
    for (record, content) in records {
        let mut snapshot = vec![0; content.len()];
        assert_eq!(
            mounted.read_retained_range(&mut recovered, &record, 0, &mut snapshot),
            Ok(content.len())
        );
        assert_eq!(snapshot, content);
    }
    assert_eq!(
        mounted.free_sectors(),
        Ok(DATA_SECTORS - (RETAINED * 2) as u64)
    );
}

#[test]
fn refusals_precede_io_and_preserve_the_generation() {
    let (mut volume, mut disk, node) = seed();
    let header = *volume.header().unwrap();
    let before = disk.operations;
    for (id, version, content, error) in [
        (
            node.id,
            node.version + 1,
            b"stale".as_slice(),
            Error::Version,
        ),
        (4, 1, b"directory", Error::IsDirectory),
        (999, 1, b"missing", Error::NotFound),
    ] {
        assert_eq!(volume.replace(&mut disk, id, version, content), Err(error));
    }
    let excessive = vec![0; format7::MAX_FILE_BYTES as usize + 1];
    assert_eq!(
        volume.replace(&mut disk, node.id, node.version, &excessive),
        Err(Error::Size)
    );
    assert_eq!(disk.operations, before);
    assert_eq!(*volume.header().unwrap(), header);
    assert_eq!(volume.stat(node.id), Ok(node));
}

#[test]
fn ordinary_payload_does_not_overwrite_an_open_stages_reservation() {
    let (mut volume, mut disk, node) = seed();
    let other = volume.create(&mut disk, 4, b"staged", Kind::File).unwrap();
    let identity = WriteIdentity7 {
        subject: 9,
        workspace: 4,
        object: other.id,
        instance: 1,
        retry_epoch: 1,
        retry_key: 1,
    };
    let mut stage = volume
        .open_stage(identity, other.version, 512, Stage7Kind::Tracked)
        .unwrap();
    volume
        .stage_write(&mut disk, &mut stage, &[0x51; 512])
        .unwrap();
    let ordinary = volume
        .replace(&mut disk, node.id, node.version, &[0x73; 512])
        .unwrap();
    volume.finish_tracked(&mut disk, &mut stage).unwrap();
    assert_eq!(bytes(&volume, &mut disk, ordinary), vec![0x73; 512]);
    let staged = volume.stat(other.id).unwrap();
    assert_eq!(bytes(&volume, &mut disk, staged), vec![0x51; 512]);
    assert_eq!(volume.free_sectors(), Ok(DATA_SECTORS - 2));
}

#[test]
fn an_intervening_ordinary_write_makes_a_same_file_stage_stale() {
    let (mut volume, mut disk, node) = seed();
    let identity = WriteIdentity7 {
        subject: 9,
        workspace: 4,
        object: node.id,
        instance: 1,
        retry_epoch: 1,
        retry_key: 1,
    };
    let mut stage = volume
        .open_stage(identity, node.version, 512, Stage7Kind::Tracked)
        .unwrap();
    volume
        .stage_write(&mut disk, &mut stage, &[0x51; 512])
        .unwrap();
    let ordinary = volume
        .replace(&mut disk, node.id, node.version, &[0x73; 512])
        .unwrap();
    let before = disk.operations;
    assert_eq!(
        volume.finish_tracked(&mut disk, &mut stage),
        Err(Error::Version)
    );
    assert_eq!(disk.operations, before);
    assert_eq!(volume.open_stages(), 0);
    assert_eq!(bytes(&volume, &mut disk, ordinary), vec![0x73; 512]);
    assert_eq!(volume.free_sectors(), Ok(DATA_SECTORS - 1));
    assert!(
        volume
            .retained_records()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn read_only_and_sequence_exhaustion_refuse_before_io() {
    for (space, sequence, error) in [(1, 1, Error::ReadOnly), (4, u64::MAX, Error::Exhausted)] {
        let mut disk = Sparse::default();
        let mut nodes = roots();
        nodes[4] = Node7 {
            id: 5,
            parent: space as u32,
            version: 1,
            length: 0,
            kind: Kind::File,
            space,
            extents_used: 0,
            extents: [Extent::new(0, 0); MAX_EXTENTS],
            name_length: 4,
            name: name_field(b"file"),
            payload_crc32: 0,
        };
        let header = persist_generation(
            &mut disk,
            Header7 {
                next: 6,
                sequence,
                ..Header7::initial(LINEAGE)
            },
            &nodes,
            &[None; RETAINED],
            &[0; MAP_WORDS],
        );
        disk.flush().unwrap();
        let mut volume = Volume7::EMPTY;
        volume.mount_into(&mut disk).unwrap();
        let before = disk.operations;
        assert_eq!(volume.replace(&mut disk, 5, 1, b"forbidden"), Err(error));
        assert_eq!(disk.operations, before);
        assert_eq!(*volume.header().unwrap(), header);
    }
}

#[test]
fn every_payload_and_publication_cut_recovers_a_complete_version() {
    let (mut volume, mut seed_disk, original) = seed();
    let original = volume
        .replace(&mut seed_disk, original.id, original.version, &[0x31; 513])
        .unwrap();
    let seed_disk = seed_disk.recover();
    let mut probe_disk = seed_disk.recover();
    let mut probe = Volume7::EMPTY;
    probe.mount_into(&mut probe_disk).unwrap();
    let start = probe_disk.operations;
    let updated = probe
        .replace(&mut probe_disk, original.id, original.version, &[0x72; 513])
        .unwrap();
    let steps = probe_disk.operations - start;
    assert!(steps > 100);
    for cut in 0..steps {
        let mut disk = seed_disk.recover();
        let mut volume = Volume7::EMPTY;
        volume.mount_into(&mut disk).unwrap();
        disk.fail_at = Some(disk.operations + cut);
        assert_eq!(
            volume.replace(&mut disk, original.id, original.version, &[0x72; 513]),
            Err(Error::Uncertain),
            "cut {cut}"
        );
        assert_eq!(volume.header(), Err(Error::Uncertain));
        let mut durable_disk = disk.recover();
        durable_disk.fail_at = None;
        let mut durable = Volume7::EMPTY;
        durable.mount_into(&mut durable_disk).unwrap();
        assert_eq!(durable.stat(original.id), Ok(original));
        assert_eq!(
            bytes(&durable, &mut durable_disk, original),
            vec![0x31; 513]
        );
        disk.fail_at = None;
        let mut live = Volume7::EMPTY;
        live.mount_into(&mut disk).unwrap();
        let expected = if cut == steps - 1 { updated } else { original };
        assert_eq!(live.stat(original.id), Ok(expected));
        let payload = if cut == steps - 1 { 0x72 } else { 0x31 };
        assert_eq!(bytes(&live, &mut disk, expected), vec![payload; 513]);
        assert_eq!(live.free_sectors(), Ok(DATA_SECTORS - 2));
        assert!(live.retained_records().unwrap().iter().all(Option::is_none));
    }
}
