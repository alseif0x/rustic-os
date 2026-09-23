// SPDX-License-Identifier: Apache-2.0
use super::*;
use rustic_fs::WriteIdentity7;

#[test]
fn create_beyond_bootstrap_capacity_and_reuse_slots_without_reusing_ids() {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();

    let mut created = Vec::new();
    for index in 0..36 {
        let name = format!("object-{index}");
        created.push(
            volume
                .create(&mut disk, 4, name.as_bytes(), Kind::File)
                .unwrap(),
        );
    }
    assert_eq!(created.first().unwrap().id, 5);
    assert_eq!(created.last().unwrap().id, 40);
    assert_eq!(volume.header().unwrap().next, 41);
    assert_eq!(
        volume
            .nodes()
            .unwrap()
            .iter()
            .filter(|node| node.kind != Kind::Empty)
            .count(),
        40
    );

    let removed = created[0];
    volume.remove(&mut disk, removed.id).unwrap();
    assert_eq!(volume.stat(removed.id), Err(Error::NotFound));
    let replacement = volume
        .create(&mut disk, 4, b"replacement", Kind::File)
        .unwrap();
    assert_eq!(replacement.id, 41);
    assert_eq!(volume.header().unwrap().next, 42);

    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut disk).unwrap();
    assert_eq!(remounted.stat(removed.id), Err(Error::NotFound));
    assert_eq!(remounted.lookup(4, b"replacement"), Ok(replacement));
}

#[test]
fn lookup_stat_and_cursor_listing_return_verified_nodes() {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();

    let directory = volume
        .create(&mut disk, 4, b"project", Kind::Directory)
        .unwrap();
    let file = volume
        .create(&mut disk, directory.id, b"notes", Kind::File)
        .unwrap();
    assert_eq!(directory.version, 2);
    assert_eq!(file.version, 3);
    assert_eq!(volume.header().unwrap().sequence, file.version);
    assert_eq!(volume.stat(file.id), Ok(file));
    assert_eq!(volume.lookup(directory.id, b"notes"), Ok(file));
    assert_eq!(
        volume.lookup(directory.id, b"missing"),
        Err(Error::NotFound)
    );
    assert_eq!(volume.list(directory.id, 0), Ok(Some((6, file))));
    assert_eq!(volume.list(directory.id, 6), Ok(None));
    assert_eq!(volume.list(directory.id, NODES + 1), Err(Error::Invalid));
    assert_eq!(volume.list(file.id, 0), Err(Error::NotDirectory));

    let mut root_entries = Vec::new();
    let mut cursor = 0;
    while let Some((next, node)) = volume.list(0, cursor).unwrap() {
        root_entries.push(node.id);
        cursor = next;
    }
    assert_eq!(root_entries, [1, 2, 3, 4]);
}

#[test]
fn rejected_namespace_mutations_leave_state_and_disk_untouched() {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    let file = volume
        .create(&mut disk, 4, b"existing", Kind::File)
        .unwrap();
    let directory = volume
        .create(&mut disk, 4, b"nonempty", Kind::Directory)
        .unwrap();
    volume
        .create(&mut disk, directory.id, b"child", Kind::File)
        .unwrap();

    let before_header = *volume.header().unwrap();
    let before_nodes = *volume.nodes().unwrap();
    let before_map = *volume.allocation_map().unwrap();
    let before_records = *volume.retained_records().unwrap();
    let before_operations = disk.operations;

    assert_eq!(
        volume.create(&mut disk, 4, b".", Kind::File),
        Err(Error::Invalid)
    );
    assert_eq!(
        volume.create(&mut disk, 4, b"empty-kind", Kind::Empty),
        Err(Error::Invalid)
    );
    assert_eq!(
        volume.create(&mut disk, 1, b"system-write", Kind::File),
        Err(Error::ReadOnly)
    );
    assert_eq!(
        volume.create(&mut disk, 0, b"virtual-root-write", Kind::File),
        Err(Error::NotFound)
    );
    assert_eq!(
        volume.create(&mut disk, 4, b"existing", Kind::File),
        Err(Error::Exists)
    );
    assert_eq!(
        volume.create(&mut disk, 999, b"bad-parent", Kind::File),
        Err(Error::NotFound)
    );
    assert_eq!(
        volume.create(&mut disk, file.id, b"not-a-directory", Kind::File),
        Err(Error::NotDirectory)
    );
    assert_eq!(volume.remove(&mut disk, 4), Err(Error::ReadOnly));
    assert_eq!(volume.remove(&mut disk, directory.id), Err(Error::NotEmpty));
    assert_eq!(volume.remove(&mut disk, 999), Err(Error::NotFound));

    assert_eq!(disk.operations, before_operations);
    assert_eq!(*volume.header().unwrap(), before_header);
    assert_eq!(*volume.nodes().unwrap(), before_nodes);
    assert_eq!(*volume.allocation_map().unwrap(), before_map);
    assert_eq!(*volume.retained_records().unwrap(), before_records);
}

#[test]
fn remove_releases_unretained_data_and_requires_children_to_be_removed_first() {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();

    let directory = volume
        .create(&mut disk, 4, b"project", Kind::Directory)
        .unwrap();
    let file = volume
        .create(&mut disk, directory.id, b"notes", Kind::File)
        .unwrap();
    let identity = WriteIdentity7 {
        subject: 7,
        workspace: 4,
        object: file.id,
        instance: 1,
        retry_epoch: 1,
        retry_key: 12,
    };
    volume
        .replace_tracked(
            &mut disk,
            identity,
            file.version,
            b"unretained after maintenance",
        )
        .unwrap();
    assert_eq!(volume.free_sectors(), Ok(DATA_SECTORS - 1));
    volume.maintain_retention(&mut disk).unwrap();
    assert_eq!(volume.free_sectors(), Ok(DATA_SECTORS - 1));

    assert_eq!(volume.remove(&mut disk, directory.id), Err(Error::NotEmpty));
    volume.remove(&mut disk, file.id).unwrap();
    assert_eq!(volume.stat(file.id), Err(Error::NotFound));
    assert_eq!(volume.free_sectors(), Ok(DATA_SECTORS));
    volume.remove(&mut disk, directory.id).unwrap();
    assert_eq!(volume.stat(directory.id), Err(Error::NotFound));
    assert_eq!(volume.remove(&mut disk, 2), Err(Error::ReadOnly));

    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut disk).unwrap();
    assert_eq!(remounted.stat(directory.id), Err(Error::NotFound));
    assert_eq!(remounted.stat(file.id), Err(Error::NotFound));
}

#[test]
fn deleting_a_file_keeps_extents_owned_by_a_retained_receipt() {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    let file = volume
        .create(&mut disk, 4, b"retained", Kind::File)
        .unwrap();
    let identity = WriteIdentity7 {
        subject: 7,
        workspace: 4,
        object: file.id,
        instance: 1,
        retry_epoch: 1,
        retry_key: 11,
    };
    let record = volume
        .replace_tracked(&mut disk, identity, file.version, b"snapshot bytes")
        .unwrap();
    let free_before_remove = volume.free_sectors().unwrap();
    assert_eq!(free_before_remove, DATA_SECTORS - 1);

    volume.remove(&mut disk, file.id).unwrap();
    assert_eq!(volume.stat(file.id), Err(Error::NotFound));
    assert_eq!(volume.free_sectors(), Ok(free_before_remove));
    assert_eq!(
        volume.retained_records().unwrap().iter().flatten().next(),
        Some(&record)
    );

    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut disk).unwrap();
    assert_eq!(remounted.stat(file.id), Err(Error::NotFound));
    assert_eq!(remounted.free_sectors(), Ok(free_before_remove));
    assert_eq!(
        remounted
            .retained_records()
            .unwrap()
            .iter()
            .flatten()
            .next(),
        Some(&record)
    );
    let before_retry_operations = disk.operations;
    assert_eq!(
        remounted.replace_tracked(&mut disk, identity, file.version, b"snapshot bytes"),
        Ok(record)
    );
    assert_eq!(disk.operations, before_retry_operations);

    remounted.maintain_retention(&mut disk).unwrap();
    assert_eq!(remounted.free_sectors(), Ok(DATA_SECTORS));
}

#[test]
fn create_publication_cut_sweep_recovers_only_complete_generations() {
    let mut seed = Sparse::default();
    let mut provisioned = Volume7::EMPTY;
    provisioned.provision_into(&mut seed, LINEAGE).unwrap();
    let prior_header = *provisioned.header().unwrap();

    let mut probe_disk = seed.recover();
    let mut probe = Volume7::EMPTY;
    probe.mount_into(&mut probe_disk).unwrap();
    let operation_start = probe_disk.operations;
    probe
        .create(&mut probe_disk, 4, b"candidate", Kind::File)
        .unwrap();
    let publication_steps = probe_disk.operations - operation_start;
    assert!(publication_steps > 100);

    for cut in 0..publication_steps {
        let mut disk = seed.recover();
        let mut volume = Volume7::EMPTY;
        volume.mount_into(&mut disk).unwrap();
        let operation_start = disk.operations;
        disk.fail_at = Some(operation_start + cut);
        assert_eq!(
            volume.create(&mut disk, 4, b"candidate", Kind::File),
            Err(Error::Uncertain),
            "create cut {cut}"
        );
        assert_eq!(volume.header(), Err(Error::Uncertain));
        assert_eq!(disk.operations, operation_start + cut + 1);

        let mut durable_disk = disk.recover();
        durable_disk.fail_at = None;
        let mut durable = Volume7::EMPTY;
        durable.mount_into(&mut durable_disk).unwrap();
        assert_eq!(*durable.header().unwrap(), prior_header, "cut {cut}");
        assert_eq!(
            durable.lookup(4, b"candidate"),
            Err(Error::NotFound),
            "durable generation at cut {cut}"
        );

        let candidate_is_live = cut == publication_steps - 1;
        let mut live = Volume7::EMPTY;
        live.mount_into(&mut disk).unwrap();
        assert_eq!(
            live.lookup(4, b"candidate").is_ok(),
            candidate_is_live,
            "live generation at cut {cut}"
        );
        assert_eq!(
            live.header().unwrap().sequence,
            prior_header.sequence + u64::from(candidate_is_live)
        );
    }
}

#[test]
fn remove_publication_cut_sweep_preserves_old_or_new_payload_ownership() {
    for keep_receipt in [false, true] {
        let mut seed = Sparse::default();
        let mut provisioned = Volume7::EMPTY;
        provisioned.provision_into(&mut seed, LINEAGE).unwrap();
        let file = provisioned
            .create(&mut seed, 4, b"payload", Kind::File)
            .unwrap();
        let identity = WriteIdentity7 {
            subject: 7,
            workspace: 4,
            object: file.id,
            instance: 1,
            retry_epoch: 1,
            retry_key: 77,
        };
        provisioned
            .replace_tracked(&mut seed, identity, file.version, b"owned payload")
            .unwrap();
        if !keep_receipt {
            provisioned.maintain_retention(&mut seed).unwrap();
        }
        let prior_header = *provisioned.header().unwrap();
        let prior_free = provisioned.free_sectors().unwrap();
        let durable_seed = seed.recover();

        let mut probe_disk = durable_seed.recover();
        let mut probe = Volume7::EMPTY;
        probe.mount_into(&mut probe_disk).unwrap();
        let operation_start = probe_disk.operations;
        probe.remove(&mut probe_disk, file.id).unwrap();
        let publication_steps = probe_disk.operations - operation_start;
        assert!(publication_steps > 100);

        for cut in 0..publication_steps {
            let mut disk = durable_seed.recover();
            let mut volume = Volume7::EMPTY;
            volume.mount_into(&mut disk).unwrap();
            let operation_start = disk.operations;
            disk.fail_at = Some(operation_start + cut);
            assert_eq!(
                volume.remove(&mut disk, file.id),
                Err(Error::Uncertain),
                "receipt={keep_receipt}, remove cut {cut}"
            );
            assert_eq!(volume.header(), Err(Error::Uncertain));

            let mut durable_disk = disk.recover();
            durable_disk.fail_at = None;
            let mut durable = Volume7::EMPTY;
            durable.mount_into(&mut durable_disk).unwrap();
            assert_eq!(*durable.header().unwrap(), prior_header);
            assert!(durable.stat(file.id).is_ok());
            assert_eq!(durable.free_sectors(), Ok(prior_free));
            assert_eq!(
                durable.retained_records().unwrap().iter().flatten().count(),
                usize::from(keep_receipt)
            );

            let removed_is_live = cut == publication_steps - 1;
            let mut live = Volume7::EMPTY;
            live.mount_into(&mut disk).unwrap();
            assert_eq!(
                live.stat(file.id).is_ok(),
                !removed_is_live,
                "live file at receipt={keep_receipt}, cut {cut}"
            );
            assert_eq!(
                live.free_sectors().unwrap(),
                if removed_is_live && !keep_receipt {
                    DATA_SECTORS
                } else {
                    prior_free
                },
                "live allocation at receipt={keep_receipt}, cut {cut}"
            );
            assert_eq!(
                live.retained_records().unwrap().iter().flatten().count(),
                usize::from(keep_receipt)
            );
        }
    }
}

#[test]
fn identity_watermark_can_reach_but_never_wraps_past_exhaustion() {
    let mut disk = Sparse::default();
    let (nodes, records, map, initial) = empty_generation(0, 1);
    let header = persist_generation(
        &mut disk,
        Header7 {
            next: u32::MAX - 1,
            ..initial
        },
        &nodes,
        &records,
        &map,
    );
    disk.flush().unwrap();

    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    assert_eq!(volume.header().unwrap().next, u32::MAX - 1);
    let last = volume
        .create(&mut disk, 4, b"last-valid", Kind::File)
        .unwrap();
    assert_eq!(last.id, u32::MAX - 1);
    assert_eq!(volume.header().unwrap().next, u32::MAX);

    let before_header = *volume.header().unwrap();
    let before_nodes = *volume.nodes().unwrap();
    let before_operations = disk.operations;
    assert_eq!(
        volume.create(&mut disk, 4, b"exhausted", Kind::File),
        Err(Error::Exhausted)
    );
    assert_eq!(disk.operations, before_operations);
    assert_eq!(*volume.header().unwrap(), before_header);
    assert_eq!(*volume.nodes().unwrap(), before_nodes);
    assert_eq!(volume.stat(last.id), Ok(last));

    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut disk).unwrap();
    assert_eq!(remounted.header().unwrap().next, u32::MAX);
    assert_eq!(remounted.stat(last.id), Ok(last));
    assert_eq!(header.next, u32::MAX - 1);
}

#[test]
fn full_namespace_refuses_creation_without_writes_or_identity_reuse() {
    let mut disk = Sparse::default();
    let (mut nodes, records, map, initial) = empty_generation(0, 1);
    for (slot, node) in nodes.iter_mut().enumerate().skip(4) {
        let id = slot as u32 + 1;
        let name = format!("object-{id}");
        *node = Node7 {
            id,
            parent: 4,
            version: 1,
            length: 0,
            kind: Kind::File,
            space: 4,
            extents_used: 0,
            extents: [Extent::new(0, 0); MAX_EXTENTS],
            name_length: name.len() as u8,
            name: name_field(name.as_bytes()),
            payload_crc32: format7::aggregate(&[]),
        };
    }
    let prior_header = persist_generation(
        &mut disk,
        Header7 {
            next: NODES as u32 + 1,
            ..initial
        },
        &nodes,
        &records,
        &map,
    );
    disk.flush().unwrap();

    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    let before_operations = disk.operations;
    assert_eq!(
        volume.create(&mut disk, 4, b"too-many", Kind::File),
        Err(Error::Full)
    );
    assert_eq!(disk.operations, before_operations);
    assert_eq!(*volume.header().unwrap(), prior_header);
    assert_eq!(*volume.nodes().unwrap(), nodes);
}

#[test]
fn sequence_exhaustion_refuses_namespace_mutation_before_io() {
    let mut disk = Sparse::default();
    let (mut nodes, records, map, initial) = empty_generation(0, u64::MAX);
    nodes[4] = Node7 {
        id: 5,
        parent: 4,
        version: 1,
        length: 0,
        kind: Kind::File,
        space: 4,
        extents_used: 0,
        extents: [Extent::new(0, 0); MAX_EXTENTS],
        name_length: 5,
        name: name_field(b"child"),
        payload_crc32: format7::aggregate(&[]),
    };
    let prior_header = persist_generation(
        &mut disk,
        Header7 {
            sequence: u64::MAX,
            next: 6,
            ..initial
        },
        &nodes,
        &records,
        &map,
    );
    disk.flush().unwrap();

    let mut volume = Volume7::EMPTY;
    volume.mount_into(&mut disk).unwrap();
    let before_operations = disk.operations;
    assert_eq!(
        volume.create(&mut disk, 4, b"exhausted-sequence", Kind::File),
        Err(Error::Exhausted)
    );
    assert_eq!(volume.remove(&mut disk, 5), Err(Error::Exhausted));
    assert_eq!(disk.operations, before_operations);
    assert_eq!(*volume.header().unwrap(), prior_header);
    assert_eq!(*volume.nodes().unwrap(), nodes);
}

struct PersistThenFailFinalFlush<'a> {
    disk: &'a mut Sparse,
    flushes: usize,
}

impl Disk for PersistThenFailFinalFlush<'_> {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        self.disk.read(sector, bytes)
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        self.disk.write(sector, bytes)
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.flushes += 1;
        self.disk.flush()?;
        if self.flushes == 2 {
            Err(Error::Io)
        } else {
            Ok(())
        }
    }
}

#[test]
fn remove_final_flush_error_after_durable_header_recovers_new_generation() {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    let file = volume.create(&mut disk, 4, b"payload", Kind::File).unwrap();
    let identity = WriteIdentity7 {
        subject: 7,
        workspace: 4,
        object: file.id,
        instance: 1,
        retry_epoch: 1,
        retry_key: 77,
    };
    volume
        .replace_tracked(&mut disk, identity, file.version, b"owned payload")
        .unwrap();
    volume.maintain_retention(&mut disk).unwrap();
    assert_eq!(volume.free_sectors(), Ok(DATA_SECTORS - 1));

    let result = {
        let mut device = PersistThenFailFinalFlush {
            disk: &mut disk,
            flushes: 0,
        };
        volume.remove(&mut device, file.id)
    };
    assert_eq!(result, Err(Error::Uncertain));
    assert_eq!(volume.header(), Err(Error::Uncertain));

    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut disk).unwrap();
    assert_eq!(remounted.stat(file.id), Err(Error::NotFound));
    assert_eq!(remounted.free_sectors(), Ok(DATA_SECTORS));
}
