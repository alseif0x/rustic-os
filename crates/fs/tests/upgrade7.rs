// SPDX-License-Identifier: Apache-2.0
//! Out-of-place v5 -> v7 conversion on sparse disposable disks.
mod support;

use rustic_fs::format7::{self, RecordState};
use rustic_fs::{
    AdmissionState, Disk, Error, Kind, PreventionReason, Replacement, Retry, Volume, Volume7,
    upgrade_v5_to_v7,
};
use support::Sparse;

const LINEAGE: [u8; 16] = [0x5a; 16];
const FOREIGN_LINEAGE: [u8; 16] = [0x23; 16];

#[derive(Default)]
struct TornHeaderTarget {
    disk: Sparse,
    torn_header: bool,
}

impl Disk for TornHeaderTarget {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        self.disk.read(sector, bytes)
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        if sector == format7::header_sector(0) && !self.torn_header {
            self.disk.operations += 1;
            let mut partial = self.disk.live.get(&sector).copied().unwrap_or([0; 512]);
            partial[..256].copy_from_slice(&bytes[..256]);
            self.disk.live.insert(sector, partial);
            self.torn_header = true;
            return Err(Error::Io);
        }
        self.disk.write(sector, bytes)
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.disk.flush()
    }
}

struct FailAfterDurableFlush {
    disk: Sparse,
    flushes: usize,
    fail_after_flush: usize,
}

impl Disk for FailAfterDurableFlush {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        self.disk.read(sector, bytes)
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        self.disk.write(sector, bytes)
    }

    fn flush(&mut self) -> Result<(), Error> {
        let current = self.flushes;
        self.flushes += 1;
        self.disk.flush()?;
        if current == self.fail_after_flush {
            Err(Error::Io)
        } else {
            Ok(())
        }
    }
}

fn retry(key: u64) -> Retry {
    Retry {
        lineage: LINEAGE,
        epoch: 1,
        key,
    }
}

fn source_with_older_direct_snapshot() -> Sparse {
    let mut disk = Sparse::default();
    let mut volume = Volume::initialize(&mut disk).unwrap();
    let file = volume.create(&mut disk, 4, b"file", Kind::File).unwrap();
    let file = volume
        .replace(&mut disk, file.id, file.version, b"initial")
        .unwrap();
    volume.enable_recovery(&mut disk, LINEAGE).unwrap();
    volume.enable_operations(&mut disk).unwrap();
    let committed = volume
        .replace_scoped(
            &mut disk,
            9,
            0,
            Replacement {
                workspace: 4,
                retry: retry(31),
                id: file.id,
                version: file.version,
            },
            b"retained snapshot",
        )
        .unwrap();
    volume
        .replace(&mut disk, file.id, committed.committed, b"new live bytes")
        .unwrap();
    disk
}

fn read_runs(disk: &mut Sparse, runs: &[rustic_fs::Extent], length: usize) -> Vec<u8> {
    let mut output = vec![0; length];
    let mut copied = 0;
    for run in runs {
        for offset in 0..run.sectors {
            if copied == length {
                return output;
            }
            let mut block = [0; 512];
            disk.read(format7::PAYLOAD_SECTOR + run.start + offset, &mut block)
                .unwrap();
            let count = (length - copied).min(block.len());
            output[copied..copied + count].copy_from_slice(&block[..count]);
            copied += count;
        }
    }
    assert_eq!(copied, length);
    output
}

fn write_envelope(disk: &mut Sparse, lineage: [u8; 16]) {
    let mut bytes = [0; 512];
    bytes[..8].copy_from_slice(b"RUSTVOL1");
    bytes[8..24].copy_from_slice(&lineage);
    let checksum = format7::aggregate(&bytes);
    bytes[24..28].copy_from_slice(&checksum.to_le_bytes());
    disk.write(1, &bytes).unwrap();
    disk.flush().unwrap();
}

fn selected_v5_bank(disk: &mut Sparse) -> u8 {
    let mut left = [0; 512];
    let mut right = [0; 512];
    disk.read(8, &mut left).unwrap();
    disk.read(13, &mut right).unwrap();
    let left_sequence = u64::from_le_bytes(left[12..20].try_into().unwrap());
    let right_sequence = u64::from_le_bytes(right[12..20].try_into().unwrap());
    u8::from(right_sequence > left_sequence)
}

fn corrupt_live_file(disk: &mut Sparse, slot: usize) {
    let bank = selected_v5_bank(disk);
    let mut table = [0; 512];
    disk.read(8 + u64::from(bank) * 5 + 1, &mut table).unwrap();
    let node_bank = table[slot * 64 + 2];
    let data_sector = 32 + slot as u64 * 4 + u64::from(node_bank) * 2;
    disk.corrupt(data_sector, 0);
}

fn lower_live_version_without_breaking_v5_checksums(disk: &mut Sparse, slot: usize, version: u64) {
    let bank = selected_v5_bank(disk);
    let header_sector = 8 + u64::from(bank) * 5;
    let mut header = [0; 512];
    disk.read(header_sector, &mut header).unwrap();
    let mut table = [0; 2048];
    for (index, block) in table.as_chunks_mut::<512>().0.iter_mut().enumerate() {
        disk.read(header_sector + 1 + index as u64, block).unwrap();
    }
    let node = slot * 64;
    table[node + 16..node + 24].copy_from_slice(&version.to_le_bytes());
    header[24..28].copy_from_slice(&format7::aggregate(&table).to_le_bytes());
    header[28..32].fill(0);
    let checksum = format7::aggregate(&header);
    header[28..32].copy_from_slice(&checksum.to_le_bytes());
    for (index, block) in table.as_chunks::<512>().0.iter().enumerate() {
        disk.write(header_sector + 1 + index as u64, block).unwrap();
    }
    disk.write(header_sector, &header).unwrap();
    disk.flush().unwrap();
}

fn replace_live_payload_with_valid_checksum(disk: &mut Sparse, slot: usize, bytes: &[u8]) {
    assert!(bytes.len() <= 512);
    let bank = selected_v5_bank(disk);
    let header_sector = 8 + u64::from(bank) * 5;
    let mut header = [0; 512];
    disk.read(header_sector, &mut header).unwrap();
    let mut table = [0; 2048];
    for (index, block) in table.as_chunks_mut::<512>().0.iter_mut().enumerate() {
        disk.read(header_sector + 1 + index as u64, block).unwrap();
    }

    let node = slot * 64;
    let node_bank = table[node + 2];
    let mut payload = [0; 512];
    let data_sector = 32 + slot as u64 * 4 + u64::from(node_bank) * 2;
    disk.read(data_sector, &mut payload).unwrap();
    payload[..bytes.len()].copy_from_slice(bytes);
    disk.write(data_sector, &payload).unwrap();

    table[node + 24..node + 28].copy_from_slice(&format7::aggregate(bytes).to_le_bytes());
    header[24..28].copy_from_slice(&format7::aggregate(&table).to_le_bytes());
    header[28..32].fill(0);
    let checksum = format7::aggregate(&header);
    header[28..32].copy_from_slice(&checksum.to_le_bytes());
    for (index, block) in table.as_chunks::<512>().0.iter().enumerate() {
        disk.write(header_sector + 1 + index as u64, block).unwrap();
    }
    disk.write(header_sector, &header).unwrap();
    disk.flush().unwrap();
}

fn assert_refusal(source: &mut Sparse, expected_lineage: [u8; 16], expected: Error) {
    let source_live = source.live.clone();
    let source_durable = source.durable.clone();
    let source_operations = source.operations;
    let mut target = Sparse::default();
    let mut volume = Volume7::EMPTY;

    assert_eq!(
        upgrade_v5_to_v7(source, &mut target, &mut volume, expected_lineage),
        Err(expected)
    );
    assert_eq!(source.live, source_live);
    assert_eq!(source.durable, source_durable);
    assert_eq!(source.operations, source_operations);
    assert!(target.live.is_empty(), "refusal must precede target writes");
    assert!(target.durable.is_empty());
    assert_eq!(target.operations, 0);
    assert_eq!(volume.header(), Err(Error::Uncertain));
}

#[test]
fn conversion_preserves_live_identity_direct_receipt_unknown_cause_and_snapshots() {
    let mut source_disk = Sparse::default();
    let mut source = Volume::initialize(&mut source_disk).unwrap();
    let file = source
        .create(&mut source_disk, 4, b"record", Kind::File)
        .unwrap();
    let file = source
        .replace(&mut source_disk, file.id, file.version, b"initial")
        .unwrap();
    source.enable_recovery(&mut source_disk, LINEAGE).unwrap();
    source.enable_operations(&mut source_disk).unwrap();
    source.enable_admissions(&mut source_disk).unwrap();

    let direct = source
        .replace_scoped(
            &mut source_disk,
            9,
            0,
            Replacement {
                workspace: 4,
                retry: retry(41),
                id: file.id,
                version: file.version,
            },
            b"committed snapshot",
        )
        .unwrap();
    let current = source
        .replace(
            &mut source_disk,
            file.id,
            direct.committed,
            b"current live bytes",
        )
        .unwrap();
    let admission = source
        .admit_replace(
            &mut source_disk,
            10,
            0,
            Replacement {
                workspace: 4,
                retry: retry(42),
                id: file.id,
                version: current.version,
            },
            b"cancelled snapshot",
        )
        .unwrap();
    let cancelled = source
        .cancel_admission(&mut source_disk, 10, admission.id)
        .unwrap();
    assert_eq!(cancelled.state, AdmissionState::Cancelled);
    assert_eq!(cancelled.prevention, Some(PreventionReason::Unknown));

    let source_sequence = source.sequence();
    let source_live = source_disk.live.clone();
    let source_durable = source_disk.durable.clone();
    let source_operations = source_disk.operations;
    let mut target = Sparse::default();
    let mut volume = Volume7::EMPTY;
    upgrade_v5_to_v7(&mut source_disk, &mut target, &mut volume, LINEAGE).unwrap();
    assert_eq!(source_disk.live, source_live);
    assert_eq!(source_disk.durable, source_durable);
    assert_eq!(source_disk.operations, source_operations);
    assert!(target.operations > 0);

    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut target).unwrap();
    let header = remounted.header().unwrap();
    assert_eq!(header.lineage, LINEAGE);
    assert_eq!(header.epoch, 1);
    assert_eq!(header.sequence, source_sequence);
    assert_eq!(header.next, current.id + 1);

    let live = remounted.node(file.id).unwrap().unwrap();
    assert_eq!(live.id, file.id);
    assert_eq!(live.parent, file.parent);
    assert_eq!(live.version, current.version);
    assert_eq!(live.kind, file.kind);
    assert_eq!(live.space, file.space);
    assert_eq!(live.name(), b"record");
    assert_eq!(
        read_runs(&mut target, live.runs(), live.length as usize),
        b"current live bytes"
    );

    let records = remounted.retained_records().unwrap();
    let committed = records[0].unwrap();
    assert_eq!(committed.state, RecordState::DirectCommitted);
    assert_eq!(committed.subject, 9);
    assert_eq!(committed.workspace, 4);
    assert_eq!(committed.object, file.id);
    assert_eq!(committed.instance, direct.committed);
    assert_eq!(committed.retry_epoch, direct.retry.epoch);
    assert_eq!(committed.retry_key, direct.retry.key);
    assert_eq!(committed.previous, direct.previous);
    assert_eq!(committed.committed, direct.committed);
    assert_eq!(committed.terminal, direct.committed);
    assert_eq!(
        read_runs(&mut target, committed.runs(), committed.length as usize),
        b"committed snapshot"
    );
    assert_ne!(committed.extents[0], live.extents[0]);

    let cancelled_record = records[1].unwrap();
    assert_eq!(cancelled_record.state, RecordState::Cancelled);
    assert_eq!(cancelled_record.prevention, Some(PreventionReason::Unknown));
    assert_eq!(cancelled_record.admission_number, admission.id.number);
    assert_eq!(cancelled_record.terminal, cancelled.terminal);
    assert_eq!(cancelled_record.committed, 0);
    assert_eq!(
        read_runs(
            &mut target,
            cancelled_record.runs(),
            cancelled_record.length as usize
        ),
        b"cancelled snapshot"
    );
    assert_ne!(cancelled_record.extents[0], live.extents[0]);
    assert_ne!(cancelled_record.extents[0], committed.extents[0]);
}

#[test]
fn current_direct_commit_aliases_the_exact_live_extents() {
    let mut source_disk = Sparse::default();
    let mut source = Volume::initialize(&mut source_disk).unwrap();
    let file = source
        .create(&mut source_disk, 4, b"current", Kind::File)
        .unwrap();
    source.enable_recovery(&mut source_disk, LINEAGE).unwrap();
    source.enable_operations(&mut source_disk).unwrap();
    let committed = source
        .replace_scoped(
            &mut source_disk,
            9,
            0,
            Replacement {
                workspace: 4,
                retry: retry(32),
                id: file.id,
                version: file.version,
            },
            b"same live and retained bytes",
        )
        .unwrap();
    let source_live = source_disk.live.clone();
    let source_operations = source_disk.operations;
    let mut target = Sparse::default();
    let mut volume = Volume7::EMPTY;

    upgrade_v5_to_v7(&mut source_disk, &mut target, &mut volume, LINEAGE).unwrap();
    assert_eq!(source_disk.live, source_live);
    assert_eq!(source_disk.operations, source_operations);

    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut target).unwrap();
    let live = remounted.node(file.id).unwrap().unwrap();
    let receipt = remounted.retained_records().unwrap()[0].unwrap();
    assert_eq!(receipt.state, RecordState::DirectCommitted);
    assert_eq!(receipt.committed, committed.committed);
    assert_eq!(receipt.runs(), live.runs());
    assert_eq!(
        read_runs(&mut target, receipt.runs(), receipt.length as usize),
        b"same live and retained bytes"
    );
}

#[test]
fn current_admitted_commit_aliases_the_exact_live_extents() {
    let mut source_disk = Sparse::default();
    let mut source = Volume::initialize(&mut source_disk).unwrap();
    let file = source
        .create(&mut source_disk, 4, b"admitted", Kind::File)
        .unwrap();
    source.enable_recovery(&mut source_disk, LINEAGE).unwrap();
    source.enable_operations(&mut source_disk).unwrap();
    source.enable_admissions(&mut source_disk).unwrap();
    let admitted = source
        .admit_replace(
            &mut source_disk,
            10,
            0,
            Replacement {
                workspace: 4,
                retry: retry(33),
                id: file.id,
                version: file.version,
            },
            b"admitted live bytes",
        )
        .unwrap();
    let mut execution = source
        .prepare_admitted(&mut source_disk, 10, admitted.id)
        .unwrap();
    while execution.result().is_none() {
        execution.advance().unwrap();
    }
    let receipt = execution.result().unwrap();
    drop(execution);
    let source_live = source_disk.live.clone();
    let source_operations = source_disk.operations;
    let mut target = Sparse::default();
    let mut volume = Volume7::EMPTY;

    upgrade_v5_to_v7(&mut source_disk, &mut target, &mut volume, LINEAGE).unwrap();
    assert_eq!(source_disk.live, source_live);
    assert_eq!(source_disk.operations, source_operations);

    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut target).unwrap();
    let live = remounted.node(file.id).unwrap().unwrap();
    let record = remounted.retained_records().unwrap()[0].unwrap();
    assert_eq!(record.state, RecordState::AdmittedCommitted);
    assert_eq!(record.committed, receipt.committed);
    assert_eq!(record.runs(), live.runs());
    assert_eq!(
        read_runs(&mut target, record.runs(), record.length as usize),
        b"admitted live bytes"
    );
}

#[test]
fn open_admission_keeps_its_candidate_snapshot_separate_from_live_data() {
    let mut source_disk = Sparse::default();
    let mut source = Volume::initialize(&mut source_disk).unwrap();
    let file = source
        .create(&mut source_disk, 4, b"pending", Kind::File)
        .unwrap();
    source
        .replace(
            &mut source_disk,
            file.id,
            file.version,
            b"live before admission",
        )
        .unwrap();
    let current = source.stat(file.id).unwrap();
    source.enable_recovery(&mut source_disk, LINEAGE).unwrap();
    source.enable_operations(&mut source_disk).unwrap();
    source.enable_admissions(&mut source_disk).unwrap();
    source
        .admit_replace(
            &mut source_disk,
            10,
            0,
            Replacement {
                workspace: 4,
                retry: retry(34),
                id: file.id,
                version: current.version,
            },
            b"pending candidate bytes",
        )
        .unwrap();
    let mut target = Sparse::default();
    let mut volume = Volume7::EMPTY;

    upgrade_v5_to_v7(&mut source_disk, &mut target, &mut volume, LINEAGE).unwrap();
    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut target).unwrap();
    let live = remounted.node(file.id).unwrap().unwrap();
    let record = remounted.retained_records().unwrap()[0].unwrap();
    assert_eq!(record.state, RecordState::Admitted);
    assert_eq!(record.committed, 0);
    assert_ne!(record.runs(), live.runs());
    assert_eq!(
        read_runs(&mut target, record.runs(), record.length as usize),
        b"pending candidate bytes"
    );
    assert_eq!(
        read_runs(&mut target, live.runs(), live.length as usize),
        b"live before admission"
    );
}

#[test]
fn explicit_cancellation_cause_survives_conversion() {
    let mut source_disk = Sparse::default();
    let mut source = Volume::initialize(&mut source_disk).unwrap();
    let file = source
        .create(&mut source_disk, 4, b"cause", Kind::File)
        .unwrap();
    source.enable_recovery(&mut source_disk, LINEAGE).unwrap();
    source.enable_operations(&mut source_disk).unwrap();
    source.enable_admissions(&mut source_disk).unwrap();
    source.enable_prevention_reasons(&mut source_disk).unwrap();
    let admitted = source
        .admit_replace(
            &mut source_disk,
            10,
            0,
            Replacement {
                workspace: 4,
                retry: retry(35),
                id: file.id,
                version: file.version,
            },
            b"authority-loss snapshot",
        )
        .unwrap();
    let mut cancellation = source
        .prepare_prevention(
            &mut source_disk,
            10,
            admitted.id,
            PreventionReason::AuthorityLost,
        )
        .unwrap();
    while cancellation.result().is_none() {
        cancellation.advance().unwrap();
    }
    drop(cancellation);
    let mut target = Sparse::default();
    let mut volume = Volume7::EMPTY;

    upgrade_v5_to_v7(&mut source_disk, &mut target, &mut volume, LINEAGE).unwrap();
    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut target).unwrap();
    let record = remounted.retained_records().unwrap()[0].unwrap();
    assert_eq!(record.state, RecordState::Cancelled);
    assert_eq!(record.prevention, Some(PreventionReason::AuthorityLost));
    assert_eq!(
        read_runs(&mut target, record.runs(), record.length as usize),
        b"authority-loss snapshot"
    );
}

#[test]
fn no_recovery_uses_the_expected_lineage_and_epoch_one() {
    let mut source_disk = Sparse::default();
    let mut source = Volume::initialize(&mut source_disk).unwrap();
    let empty = source
        .create(&mut source_disk, 4, b"empty", Kind::File)
        .unwrap();
    let sequence = source.sequence();
    let source_live = source_disk.live.clone();
    let mut target = Sparse::default();
    let mut volume = Volume7::EMPTY;

    upgrade_v5_to_v7(&mut source_disk, &mut target, &mut volume, LINEAGE).unwrap();
    assert_eq!(source_disk.live, source_live);
    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut target).unwrap();
    assert_eq!(remounted.header().unwrap().lineage, LINEAGE);
    assert_eq!(remounted.header().unwrap().epoch, 1);
    assert_eq!(remounted.header().unwrap().sequence, sequence);
    assert_eq!(remounted.header().unwrap().next, empty.id + 1);
    let file = remounted.node(empty.id).unwrap().unwrap();
    assert_eq!(file.length, 0);
    assert_eq!(file.payload_crc32, format7::aggregate(&[]));
}

#[test]
fn legacy_envelope_supplies_identity_without_publishing_v5_recovery() {
    let mut source_disk = Sparse::default();
    let mut source = Volume::initialize(&mut source_disk).unwrap();
    source
        .create(&mut source_disk, 4, b"legacy", Kind::File)
        .unwrap();
    write_envelope(&mut source_disk, FOREIGN_LINEAGE);
    let source_live = source_disk.live.clone();
    let mut target = Sparse::default();
    let mut volume = Volume7::EMPTY;

    upgrade_v5_to_v7(&mut source_disk, &mut target, &mut volume, FOREIGN_LINEAGE).unwrap();
    assert_eq!(source_disk.live, source_live);
    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut target).unwrap();
    assert_eq!(remounted.header().unwrap().lineage, FOREIGN_LINEAGE);
    assert_eq!(remounted.header().unwrap().epoch, 1);
}

#[test]
fn foreign_envelope_lineage_is_refused_without_source_or_target_writes() {
    let mut source_disk = Sparse::default();
    Volume::initialize(&mut source_disk).unwrap();
    write_envelope(&mut source_disk, FOREIGN_LINEAGE);

    assert_refusal(&mut source_disk, LINEAGE, Error::Lineage);
}

#[test]
fn scope_less_retained_record_is_refused_without_source_or_target_writes() {
    let mut source_disk = Sparse::default();
    let mut source = Volume::initialize(&mut source_disk).unwrap();
    let file = source
        .create(&mut source_disk, 4, b"legacy", Kind::File)
        .unwrap();
    let file = source
        .replace(&mut source_disk, file.id, file.version, b"before")
        .unwrap();
    source.enable_recovery(&mut source_disk, LINEAGE).unwrap();
    source
        .replace_tracked(
            &mut source_disk,
            9,
            retry(51),
            file.id,
            file.version,
            b"after",
        )
        .unwrap();

    assert_refusal(&mut source_disk, LINEAGE, Error::Unsupported);
}

#[test]
fn foreign_recovery_lineage_is_refused_without_source_or_target_writes() {
    let mut source_disk = Sparse::default();
    let mut source = Volume::initialize(&mut source_disk).unwrap();
    source
        .create(&mut source_disk, 4, b"file", Kind::File)
        .unwrap();
    source
        .enable_recovery(&mut source_disk, FOREIGN_LINEAGE)
        .unwrap();

    assert_refusal(&mut source_disk, LINEAGE, Error::Lineage);
}

#[test]
fn corrupt_live_payload_crc_is_refused_before_target_writes() {
    let mut source_disk = Sparse::default();
    let mut source = Volume::initialize(&mut source_disk).unwrap();
    let file = source
        .create(&mut source_disk, 4, b"file", Kind::File)
        .unwrap();
    source
        .replace(&mut source_disk, file.id, file.version, b"payload")
        .unwrap();
    corrupt_live_file(&mut source_disk, 4);

    assert_refusal(&mut source_disk, LINEAGE, Error::Corrupt);
}

#[test]
fn checksum_valid_same_crc_live_bytes_cannot_alias_a_retained_snapshot() {
    const SNAPSHOT: &[u8] = b"the original committed bytes";
    // A deliberate IEEE CRC32 collision: checksum equality alone is not byte identity.
    const DIFFERENT_BYTES: &[u8] = b"uhe original committed b^\x11@\xf2";
    assert_ne!(SNAPSHOT, DIFFERENT_BYTES);
    assert_eq!(SNAPSHOT.len(), DIFFERENT_BYTES.len());
    assert_eq!(
        format7::aggregate(SNAPSHOT),
        format7::aggregate(DIFFERENT_BYTES)
    );

    let mut source_disk = Sparse::default();
    let mut source = Volume::initialize(&mut source_disk).unwrap();
    let file = source
        .create(&mut source_disk, 4, b"collision", Kind::File)
        .unwrap();
    source.enable_recovery(&mut source_disk, LINEAGE).unwrap();
    source.enable_operations(&mut source_disk).unwrap();
    source
        .replace_scoped(
            &mut source_disk,
            9,
            0,
            Replacement {
                workspace: 4,
                retry: retry(62),
                id: file.id,
                version: file.version,
            },
            SNAPSHOT,
        )
        .unwrap();
    replace_live_payload_with_valid_checksum(&mut source_disk, 4, DIFFERENT_BYTES);

    assert_refusal(&mut source_disk, LINEAGE, Error::Corrupt);
}

#[test]
fn invalid_converted_history_is_refused_before_target_writes() {
    let mut source_disk = Sparse::default();
    let mut source = Volume::initialize(&mut source_disk).unwrap();
    let file = source
        .create(&mut source_disk, 4, b"file", Kind::File)
        .unwrap();
    let file = source
        .replace(&mut source_disk, file.id, file.version, b"before")
        .unwrap();
    source.enable_recovery(&mut source_disk, LINEAGE).unwrap();
    source.enable_operations(&mut source_disk).unwrap();
    source
        .replace_scoped(
            &mut source_disk,
            9,
            0,
            Replacement {
                workspace: 4,
                retry: retry(61),
                id: file.id,
                version: file.version,
            },
            b"committed",
        )
        .unwrap();
    lower_live_version_without_breaking_v5_checksums(&mut source_disk, 4, 1);

    assert_refusal(&mut source_disk, LINEAGE, Error::Corrupt);
}

#[test]
fn equal_sequence_metadata_disagreement_is_refused_without_writes() {
    let mut source_disk = Sparse::default();
    let mut source = Volume::initialize(&mut source_disk).unwrap();
    source
        .create(&mut source_disk, 4, b"file", Kind::File)
        .unwrap();
    let active = selected_v5_bank(&mut source_disk);
    let other = 1 - active;
    for offset in 0..5 {
        let mut block = [0; 512];
        source_disk
            .read(8 + u64::from(active) * 5 + offset, &mut block)
            .unwrap();
        source_disk
            .write(8 + u64::from(other) * 5 + offset, &block)
            .unwrap();
    }
    let other_header = 8 + u64::from(other) * 5;
    let mut header = [0; 512];
    source_disk.read(other_header, &mut header).unwrap();
    let next = u32::from_le_bytes(header[20..24].try_into().unwrap());
    header[20..24].copy_from_slice(&(next + 1).to_le_bytes());
    header[28..32].fill(0);
    let checksum = format7::aggregate(&header);
    header[28..32].copy_from_slice(&checksum.to_le_bytes());
    source_disk.write(other_header, &header).unwrap();
    source_disk.flush().unwrap();

    assert_refusal(&mut source_disk, LINEAGE, Error::Corrupt);
}

#[test]
fn nonzero_target_header_is_refused_without_modifying_either_disk() {
    let mut source_disk = Sparse::default();
    Volume::initialize(&mut source_disk).unwrap();
    let source_live = source_disk.live.clone();
    let mut target = Sparse::default();
    let mut existing = Volume7::EMPTY;
    existing.provision_into(&mut target, LINEAGE).unwrap();
    let target_live = target.live.clone();
    let target_durable = target.durable.clone();
    let target_operations = target.operations;
    let mut volume = Volume7::EMPTY;

    assert_eq!(
        upgrade_v5_to_v7(&mut source_disk, &mut target, &mut volume, LINEAGE),
        Err(Error::Exists)
    );
    assert_eq!(source_disk.live, source_live);
    assert_eq!(target.live, target_live);
    assert_eq!(target.durable, target_durable);
    assert_eq!(target.operations, target_operations);
    assert_eq!(volume.header(), Err(Error::Uncertain));
}

#[test]
fn every_target_write_and_flush_failure_leaves_no_mountable_head() {
    let mut source_disk = source_with_older_direct_snapshot();
    let source_live = source_disk.live.clone();
    let source_durable = source_disk.durable.clone();
    let source_operations = source_disk.operations;
    let mut complete_target = Sparse::default();
    let mut complete_volume = Volume7::EMPTY;
    upgrade_v5_to_v7(
        &mut source_disk,
        &mut complete_target,
        &mut complete_volume,
        LINEAGE,
    )
    .unwrap();
    let operation_count = complete_target.operations;
    assert!(operation_count >= format7::NODES_SECTORS as usize + 4);
    assert_eq!(source_disk.live, source_live);
    assert_eq!(source_disk.durable, source_durable);
    assert_eq!(source_disk.operations, source_operations);

    for cut in 0..operation_count {
        let mut target = Sparse {
            fail_at: Some(cut),
            ..Sparse::default()
        };
        let mut volume = Volume7::EMPTY;
        assert_eq!(
            upgrade_v5_to_v7(&mut source_disk, &mut target, &mut volume, LINEAGE),
            Err(Error::Io),
            "write/flush cut {cut}"
        );
        assert_eq!(target.operations, cut + 1);
        assert_eq!(source_disk.live, source_live);
        assert_eq!(source_disk.durable, source_durable);
        assert_eq!(source_disk.operations, source_operations);
        assert_eq!(volume.header(), Err(Error::Uncertain));

        let mut crashed = target.recover();
        crashed.fail_at = None;
        let mut remounted = Volume7::EMPTY;
        assert!(
            remounted.mount_into(&mut crashed).is_err(),
            "failed cut {cut} must not publish a mountable head"
        );
    }
}

#[test]
fn torn_migration_header_is_refused_after_remount() {
    let mut source_disk = source_with_older_direct_snapshot();
    let source_live = source_disk.live.clone();
    let source_durable = source_disk.durable.clone();
    let source_operations = source_disk.operations;
    let mut target = TornHeaderTarget::default();
    let mut volume = Volume7::EMPTY;

    assert_eq!(
        upgrade_v5_to_v7(&mut source_disk, &mut target, &mut volume, LINEAGE),
        Err(Error::Io)
    );
    assert!(target.torn_header);
    assert_eq!(source_disk.live, source_live);
    assert_eq!(source_disk.durable, source_durable);
    assert_eq!(source_disk.operations, source_operations);
    assert_eq!(volume.header(), Err(Error::Uncertain));

    let mut partial = Sparse {
        live: target.disk.live.clone(),
        durable: target.disk.durable.clone(),
        operations: 0,
        fail_at: None,
    };
    let mut remounted = Volume7::EMPTY;
    assert!(remounted.mount_into(&mut partial).is_err());
    let mut crashed = target.disk.recover();
    crashed.fail_at = None;
    assert!(remounted.mount_into(&mut crashed).is_err());
}

#[test]
fn final_flush_error_after_durability_recovers_complete_generation() {
    let mut source_disk = source_with_older_direct_snapshot();
    let source_live = source_disk.live.clone();
    let source_durable = source_disk.durable.clone();
    let source_operations = source_disk.operations;
    let mut target = FailAfterDurableFlush {
        disk: Sparse::default(),
        flushes: 0,
        fail_after_flush: 2,
    };
    let mut volume = Volume7::EMPTY;

    assert_eq!(
        upgrade_v5_to_v7(&mut source_disk, &mut target, &mut volume, LINEAGE),
        Err(Error::Io)
    );
    assert_eq!(target.flushes, 3);
    assert_eq!(source_disk.live, source_live);
    assert_eq!(source_disk.durable, source_durable);
    assert_eq!(source_disk.operations, source_operations);
    assert_eq!(volume.header(), Err(Error::Uncertain));
    assert!(target.disk.durable.contains_key(&format7::header_sector(0)));

    let mut recovered = target.disk.recover();
    recovered.fail_at = None;
    let mut remounted = Volume7::EMPTY;
    remounted.mount_into(&mut recovered).unwrap();
    assert_eq!(remounted.header().unwrap().lineage, LINEAGE);
    let node = remounted.node(5).unwrap().unwrap();
    assert_eq!(
        read_runs(&mut recovered, node.runs(), node.length as usize),
        b"new live bytes"
    );
    let record = remounted.retained_records().unwrap()[0].unwrap();
    assert_eq!(
        read_runs(&mut recovered, record.runs(), record.length as usize),
        b"retained snapshot"
    );
}
