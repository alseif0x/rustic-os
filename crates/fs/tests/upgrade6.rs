// SPDX-License-Identifier: Apache-2.0
//! Host tests for the deliberate v5 -> v6 volume upgrade (#51): what it preserves,
//! what it refuses and what a failure leaves behind.
mod support;

use rustic_fs::{
    DATA_SECTORS, Disk, Error, Kind, MAX_FILE, OBJECTS_V6, Retry, Volume, mount6, provision6,
    upgrade6,
};
use support::Sparse;

const LINEAGE: [u8; 16] = [7; 16];
/// The lineage of the provisioning envelope the last test writes by hand.
const ENVELOPE_LINEAGE: [u8; 16] = [3; 16];

fn read_back(volume: &Volume, disk: &mut Sparse, id: u32) -> Vec<u8> {
    let mut bytes = [0; MAX_FILE];
    let count = volume.read(disk, id, 0, &mut bytes).expect("read");
    bytes[..count].to_vec()
}

#[test]
fn upgrade_preserves_identity_versions_names_bytes_and_deletions() {
    let mut disk = Sparse::default();
    let mut source = Volume::initialize(&mut disk).unwrap();
    let dir = source
        .create(&mut disk, 4, b"project", Kind::Directory)
        .unwrap();
    let notes = source
        .create(&mut disk, dir.id, b"notes.txt", Kind::File)
        .unwrap();
    let notes = source
        .replace(&mut disk, notes.id, notes.version, b"migrated content")
        .unwrap();
    let empty = source.create(&mut disk, 4, b"empty", Kind::File).unwrap();
    let gone = source.create(&mut disk, 4, b"gone", Kind::File).unwrap();
    let gone = source
        .replace(&mut disk, gone.id, gone.version, b"discarded")
        .unwrap();
    source.remove(&mut disk, gone.id).unwrap();

    let upgraded = upgrade6(&mut disk, LINEAGE).expect("upgrade");
    assert_eq!(upgraded.report.source_sequence, source.sequence());
    assert_eq!(upgraded.report.files, 2);
    assert_eq!(upgraded.report.directories, 5);
    assert_eq!(upgraded.report.bytes, 16);
    // Four roots plus four creates advance the v5 allocator from 5 to 9, and a
    // v6 volume does not carry it, so the caller seeds identity from the report.
    assert_eq!(upgraded.report.next, 9);

    // The published v6 volume is what a later mount sees, after a reboot.
    let mut disk = disk.recover();
    let mounted = mount6(&mut disk).expect("mount upgraded volume");
    let file = mounted.node(notes.id).expect("file node");
    assert_eq!(file.kind, Kind::File);
    assert_eq!(file.parent, dir.id);
    assert_eq!(file.version, notes.version);
    assert_eq!(file.length as usize, notes.length as usize);
    assert_eq!(file.name(), b"notes.txt");
    let mut bytes = vec![0; file.length as usize];
    assert_eq!(
        mounted.read_file(&mut disk, file, &mut bytes),
        Ok(bytes.len())
    );
    assert_eq!(bytes, b"migrated content");

    let folder = mounted.node(dir.id).expect("directory node");
    assert_eq!(folder.kind, Kind::Directory);
    assert_eq!(folder.version, dir.version);
    assert_eq!(folder.name(), b"project");
    assert_eq!(mounted.node(empty.id).unwrap().length, 0);
    // A deleted identity is not resurrected, and only the live payload is charged.
    assert!(mounted.node(gone.id).is_none());
    assert!(mounted.node(1).is_some() && mounted.node(4).is_some());
    assert_eq!(mounted.free_sectors(), DATA_SECTORS - 1);

    // The upgrade is deliberate and one-way: a v6 volume is not an input.
    assert_eq!(upgrade6(&mut disk, LINEAGE).err(), Some(Error::Exists));
}

#[test]
fn a_migrated_volume_tracks_a_live_identity_above_the_object_capacity() {
    let mut disk = Sparse::default();
    let mut source = Volume::initialize(&mut disk).unwrap();
    source.enable_recovery(&mut disk, LINEAGE).unwrap();
    // v5 issues a monotonic identity per create while a slot lasts only as long
    // as an object is live, so cycling one record drives the next live identity
    // past the number of v6 objects without ever exceeding the v5 table.
    let mut probe = source.create(&mut disk, 4, b"probe", Kind::File).unwrap();
    while probe.id < OBJECTS_V6 as u32 {
        source.remove(&mut disk, probe.id).unwrap();
        probe = source.create(&mut disk, 4, b"probe", Kind::File).unwrap();
    }
    source.remove(&mut disk, probe.id).unwrap();
    let live = source
        .create(&mut disk, 4, b"live", Kind::File)
        .expect("live identity");
    assert_eq!(live.id, OBJECTS_V6 as u32 + 1);

    // The migration preserves the identity instead of renumbering it, and the
    // v5 watermark travels out of band because a v6 volume does not carry it.
    let mut disk = disk.recover();
    let upgraded = upgrade6(&mut disk, LINEAGE).expect("upgrade");
    assert_eq!(upgraded.report.next, live.id + 1);

    // The migrated volume tracks the live identity: identity, previous version,
    // committed version and length are published by the one commit.
    let retry = Retry {
        lineage: LINEAGE,
        epoch: 1,
        key: 21,
    };
    let mut disk = disk.recover();
    let mut volume = mount6(&mut disk).expect("mount migrated volume");
    let slot = volume
        .nodes
        .iter()
        .position(|node| node.id == live.id)
        .expect("live slot");
    let expected = volume.nodes[slot].version;
    let receipt = volume
        .write_tracked(&mut disk, slot, expected, retry, b"tracked after migration")
        .expect("tracked write");
    assert_eq!(
        (receipt.id, receipt.previous, receipt.committed),
        (live.id, expected, expected + 1)
    );
    assert_eq!(receipt.length, b"tracked after migration".len() as u32);

    // Reading that receipt back is what an identity bound against the live
    // capacity refused: the evidence is retained, not corrupt.
    let mut disk = disk.recover();
    let mut volume = mount6(&mut disk).expect("remount after reboot");
    let node = *volume.node(live.id).expect("live node survives the reboot");
    assert_eq!(node.version, receipt.committed);
    let mut bytes = vec![0; node.length as usize];
    assert_eq!(
        volume.read_file(&mut disk, &node, &mut bytes),
        Ok(bytes.len())
    );
    assert_eq!(bytes, b"tracked after migration");
    assert_eq!(volume.find_receipt(retry), Ok(Some(&receipt)));

    // A resubmission of the same operation is answered from the retained record
    // with no further write and no further flush.
    let slot = volume
        .nodes
        .iter()
        .position(|node| node.id == live.id)
        .expect("live slot");
    let operations = disk.operations;
    assert_eq!(
        volume.write_tracked(&mut disk, slot, expected, retry, b"tracked after migration"),
        Ok(receipt)
    );
    assert_eq!(disk.operations, operations);
    assert_eq!(volume.nodes[slot].version, receipt.committed);
    let mut replayed = vec![0; node.length as usize];
    assert_eq!(
        volume.read_file(&mut disk, &node, &mut replayed),
        Ok(replayed.len())
    );
    assert_eq!(replayed, b"tracked after migration");
}

#[test]
fn upgrade_refuses_while_the_volume_retains_recovery_evidence() {
    let mut disk = Sparse::default();
    let mut source = Volume::initialize(&mut disk).unwrap();
    let file = source.create(&mut disk, 4, b"atomic", Kind::File).unwrap();
    let file = source
        .replace(&mut disk, file.id, file.version, b"before")
        .unwrap();
    source.enable_recovery(&mut disk, LINEAGE).unwrap();
    let retry = Retry {
        lineage: LINEAGE,
        epoch: 1,
        key: 9,
    };
    let receipt = source
        .replace_tracked(
            &mut disk,
            u64::from(file.id),
            retry,
            file.id,
            file.version,
            b"after",
        )
        .unwrap();
    let mut disk = disk.recover();

    // The v5 record snapshots the original bytes and v6 has no such snapshot, so
    // the upgrade refuses instead of dropping evidence, and writes nothing.
    // The probe mount flushes once (its recovery boundary); the content is what
    // the refusal must leave alone.
    let before = disk.live.clone();
    let operations = disk.operations;
    assert_eq!(upgrade6(&mut disk, LINEAGE).err(), Some(Error::Unsupported));
    assert_eq!(disk.operations, operations + 1);
    assert_eq!(disk.live, before, "a refused upgrade must not write");

    let source = Volume::mount(&mut disk).expect("the v5 volume is untouched");
    assert_eq!(source.receipt(u64::from(file.id), retry), Ok(receipt));
    assert_eq!(read_back(&source, &mut disk, file.id), b"after");
    assert_eq!(mount6(&mut disk).err(), Some(Error::Corrupt));
}

#[test]
fn upgrade_refuses_a_foreign_lineage() {
    let mut disk = Sparse::default();
    let mut source = Volume::initialize(&mut disk).unwrap();
    source.enable_recovery(&mut disk, LINEAGE).unwrap();
    let mut disk = disk.recover();
    assert_eq!(upgrade6(&mut disk, [9; 16]).err(), Some(Error::Lineage));
    assert!(Volume::mount(&mut disk).is_ok());
}

#[test]
fn a_failure_while_staging_leaves_the_v5_volume_mountable() {
    let mut disk = Sparse::default();
    let mut source = Volume::initialize(&mut disk).unwrap();
    let file = source.create(&mut disk, 4, b"notes", Kind::File).unwrap();
    let file = source
        .replace(&mut disk, file.id, file.version, b"keep me")
        .unwrap();
    let mut disk = disk.recover();

    // Payload is staged above the v5 layout before anything is published, so a
    // failed write there costs nothing. The migration first probes for a v6
    // volume, and that mount flushes once, so the payload write is operation 1.
    disk.fail_at = Some(1);
    assert_eq!(upgrade6(&mut disk, LINEAGE).err(), Some(Error::Io));
    disk.fail_at = None;
    let source = Volume::mount(&mut disk).expect("v5 still mounts");
    assert_eq!(read_back(&source, &mut disk, file.id), b"keep me");
    assert_eq!(mount6(&mut disk).err(), Some(Error::Corrupt));
}

#[test]
fn a_torn_publish_never_presents_a_v6_volume() {
    let mut disk = Sparse::default();
    let mut source = Volume::initialize(&mut disk).unwrap();
    let file = source.create(&mut disk, 4, b"notes", Kind::File).unwrap();
    source
        .replace(&mut disk, file.id, file.version, b"keep me")
        .unwrap();
    let mut disk = disk.recover();

    // The payload write and the v5 bank-1 clear succeed, then the first
    // structure write of the publish fails: the header still carries the v5
    // magic, so no half v6 volume mounts. What is left is the v5 header over
    // data the structure writes have begun to overwrite, which is why the
    // migration needs a caller-owned backup.
    // Operation 0 is the migration's probe flush, 1 the payload write and 2 the
    // bank clear, so 4 is the first structure write of the publish.
    disk.fail_at = Some(4);
    assert_eq!(upgrade6(&mut disk, LINEAGE).err(), Some(Error::Uncertain));
    disk.fail_at = None;
    assert_eq!(mount6(&mut disk).err(), Some(Error::Corrupt));
}

#[test]
fn exactly_one_layout_is_mountable_after_the_upgrade() {
    // A provisioning envelope makes a v5 mount write: it enables recovery on the
    // mounted volume. If the upgrade left a valid v5 bank, that mount would
    // publish a v5 header over the v6 one, so both v5 banks must be gone.
    const ENVELOPE_CRC: [u8; 4] = [80, 55, 29, 127]; // crc32 of the rest, lineage 3
    let mut disk = Sparse::default();
    let mut envelope = [0; 512];
    envelope[..8].copy_from_slice(b"RUSTVOL1");
    envelope[8..24].fill(3);
    envelope[24..28].copy_from_slice(&ENVELOPE_CRC);
    disk.write(1, &envelope).unwrap();
    let mut source = Volume::initialize(&mut disk).unwrap();
    let file = source.create(&mut disk, 4, b"notes", Kind::File).unwrap();
    source
        .replace(&mut disk, file.id, file.version, b"keep me")
        .unwrap();
    assert!(Volume::mount(&mut disk).is_ok());

    let mut disk = disk.recover();
    assert!(upgrade6(&mut disk, ENVELOPE_LINEAGE).is_ok());
    let mut disk = disk.recover();
    assert_eq!(Volume::mount(&mut disk).err(), Some(Error::Corrupt));
    let mut header = [0; 512];
    disk.read(8, &mut header).unwrap();
    assert_eq!(&header[..8], b"RUSTFS2\0");
    let mut alternate = [0; 512];
    disk.read(13, &mut alternate).unwrap();
    assert_eq!(alternate, [0; 512]);
    assert!(mount6(&mut disk).is_ok());
}

#[test]
fn an_unformatted_disk_is_not_migratable() {
    let mut disk = Sparse::default();
    assert_eq!(upgrade6(&mut disk, LINEAGE).err(), Some(Error::Empty));
    assert!(provision6(&mut disk, LINEAGE).is_ok());
}
