// SPDX-License-Identifier: Apache-2.0
//! Whole-generation structural validation contracts for format7.

use rustic_fs::format7::{
    Header7, MAP_WORDS, MAX_EXTENTS, NAME_BYTES, NEXT_MIN, NODES, Node7, RETAINED, Record7,
    RecordState, validate_generation,
};
use rustic_fs::{Error, Extent, Kind, PreventionReason};

#[derive(Clone)]
struct Generation {
    header: Header7,
    nodes: [Node7; NODES],
    receipts: [Option<Record7>; RETAINED],
    map: [u64; MAP_WORDS],
}

#[derive(Clone, Copy)]
struct Payload<'a> {
    length: u32,
    crc32: u32,
    runs: &'a [(u64, u64)],
}

impl<'a> Payload<'a> {
    const fn new(length: u32, crc32: u32, runs: &'a [(u64, u64)]) -> Self {
        Self {
            length,
            crc32,
            runs,
        }
    }
}

fn formatted() -> Generation {
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
    Generation {
        header: Header7::initial([0x5a; 16]),
        nodes,
        receipts: [None; RETAINED],
        map: [0; MAP_WORDS],
    }
}

fn name_field(name: &[u8]) -> [u8; NAME_BYTES] {
    let mut field = [0; NAME_BYTES];
    field[..name.len()].copy_from_slice(name);
    field
}

fn file(id: u32, parent: u32, space: u8, name: &[u8], version: u64, payload: Payload<'_>) -> Node7 {
    let mut extents = [Extent::new(0, 0); MAX_EXTENTS];
    for (target, (start, sectors)) in extents.iter_mut().zip(payload.runs) {
        *target = Extent::new(*start, *sectors);
    }
    Node7 {
        id,
        parent,
        version,
        length: payload.length,
        kind: Kind::File,
        space,
        extents_used: payload.runs.len() as u8,
        extents,
        name_length: name.len() as u8,
        name: name_field(name),
        payload_crc32: payload.crc32,
    }
}

fn directory(id: u32, parent: u32, space: u8, name: &[u8], version: u64) -> Node7 {
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

fn direct_record(
    object: u32,
    workspace: u32,
    retry_key: u64,
    epoch: u64,
    previous: u64,
    committed: u64,
    payload: Payload<'_>,
) -> Record7 {
    let mut extents = [Extent::new(0, 0); MAX_EXTENTS];
    for (target, (start, sectors)) in extents.iter_mut().zip(payload.runs) {
        *target = Extent::new(*start, *sectors);
    }
    Record7 {
        subject: 9,
        workspace,
        object,
        instance: committed,
        retry_epoch: epoch,
        retry_key,
        previous,
        committed,
        admission_number: 0,
        terminal: committed,
        length: payload.length,
        payload_crc32: payload.crc32,
        state: RecordState::DirectCommitted,
        prevention: None,
        extents_used: payload.runs.len() as u8,
        extents,
    }
}

fn record_in_state(mut record: Record7, state: RecordState) -> Record7 {
    match state {
        RecordState::DirectCommitted => (),
        RecordState::Admitted => {
            record.instance = 4;
            record.committed = 0;
            record.admission_number = 4;
            record.terminal = 0;
            record.state = RecordState::Admitted;
        }
        RecordState::Cancelled => {
            record.instance = 4;
            record.committed = 0;
            record.admission_number = 4;
            record.terminal = 5;
            record.state = RecordState::Cancelled;
            record.prevention = Some(PreventionReason::Requested);
        }
        RecordState::AdmittedCommitted => {
            record.instance = 4;
            record.committed = 5;
            record.admission_number = 4;
            record.terminal = 5;
            record.state = RecordState::AdmittedCommitted;
        }
    }
    record
}

fn with_record(record: Record7) -> Generation {
    let mut generation = formatted();
    generation.header.epoch = record.retry_epoch;
    generation.header.sequence = 9;
    generation.header.next = 7;
    generation.receipts[0] = Some(record);
    generation
}

fn receipt_for_state(state: RecordState) -> Record7 {
    record_in_state(
        direct_record(5, 6, 11, 2, 3, 4, Payload::new(512, 77, &[(20, 1)])),
        state,
    )
}

fn allocated(map: &mut [u64; MAP_WORDS], start: u64, sectors: u64) {
    for sector in start..start + sectors {
        map[sector as usize / 64] |= 1u64 << (sector % 64);
    }
}

fn validate(generation: &Generation) -> Result<(), Error> {
    let mut scratch = [0; MAP_WORDS];
    validate_generation(
        &generation.header,
        &generation.nodes,
        &generation.receipts,
        &generation.map,
        &mut scratch,
    )
}

fn rejects(generation: &Generation) {
    assert_eq!(validate(generation), Err(Error::Corrupt));
}

#[test]
fn formatted_roots_and_empty_files_validate_without_receipts() {
    let mut generation = formatted();
    assert_eq!(generation.header.next, NEXT_MIN);
    assert_eq!(validate(&generation), Ok(()));

    generation.nodes[4] = file(5, 1, 1, b"empty", 1, Payload::new(0, 0, &[]));
    generation.header.next = 6;
    assert_eq!(validate(&generation), Ok(()));

    // Node-local validation accepts a CRC field for an empty file, but a whole
    // generation must identify the empty payload with CRC-32(empty).
    generation.nodes[4].payload_crc32 = 1;
    rejects(&generation);
}

#[test]
fn roots_parentage_types_spaces_cycles_and_names_are_global_invariants() {
    let mut broken = formatted();
    broken.nodes[3] = Node7::EMPTY;
    rejects(&broken);

    let mut broken = formatted();
    broken.nodes[0].name = name_field(b"other");
    broken.nodes[0].name_length = 5;
    assert!(broken.nodes[0].validate().is_ok());
    rejects(&broken);

    let mut broken = formatted();
    broken.nodes[1].space = 3;
    rejects(&broken);

    let mut broken = formatted();
    broken.nodes[4] = directory(5, 0, 1, b"extra", 1);
    broken.header.next = 6;
    rejects(&broken);

    let mut broken = formatted();
    broken.nodes[4] = file(5, 42, 1, b"orphan", 1, Payload::new(0, 0, &[]));
    broken.header.next = 6;
    rejects(&broken);

    let mut broken = formatted();
    broken.nodes[4] = file(5, 1, 1, b"parent", 1, Payload::new(0, 0, &[]));
    broken.nodes[5] = file(6, 5, 1, b"child", 1, Payload::new(0, 0, &[]));
    broken.header.next = 7;
    rejects(&broken);

    let mut broken = formatted();
    broken.nodes[4] = directory(5, 1, 1, b"parent", 1);
    broken.nodes[5] = file(6, 5, 2, b"child", 1, Payload::new(0, 0, &[]));
    broken.header.next = 7;
    assert!(broken.nodes[4].validate().is_ok());
    assert!(broken.nodes[5].validate().is_ok());
    rejects(&broken);

    let mut broken = formatted();
    broken.nodes[4] = directory(5, 6, 1, b"left", 1);
    broken.nodes[5] = directory(6, 5, 1, b"right", 1);
    broken.header.next = 7;
    rejects(&broken);

    let mut broken = formatted();
    broken.nodes[4] = file(5, 1, 1, b"first", 1, Payload::new(0, 0, &[]));
    broken.nodes[5] = file(5, 1, 1, b"second", 1, Payload::new(0, 0, &[]));
    broken.header.next = 6;
    rejects(&broken);

    let mut broken = formatted();
    broken.nodes[4] = file(5, 1, 1, b"same", 1, Payload::new(0, 0, &[]));
    broken.nodes[5] = file(6, 1, 1, b"same", 1, Payload::new(0, 0, &[]));
    broken.header.next = 7;
    rejects(&broken);

    let mut broken = formatted();
    broken.nodes[4] = file(5, 1, 1, b"newer", 2, Payload::new(0, 0, &[]));
    broken.header.next = 6;
    rejects(&broken);

    let mut broken = formatted();
    broken.nodes[4] = file(5, 1, 1, b"unallocated", 1, Payload::new(0, 0, &[]));
    rejects(&broken);
}

#[test]
fn identities_use_the_watermark_not_the_node_table_capacity() {
    let mut generation = formatted();
    generation.header.epoch = 2;
    generation.header.sequence = 9;
    generation.header.next = 304;
    generation.nodes[4] = file(300, 4, 4, b"high", 9, Payload::new(0, 0, &[]));
    // Retained scopes are historical and need not name live objects or
    // workspaces, while both identities still obey the monotonic watermark.
    generation.receipts[0] = Some(direct_record(
        302,
        303,
        11,
        2,
        3,
        4,
        Payload::new(0, 0, &[]),
    ));
    assert_eq!(validate(&generation), Ok(()));

    generation.header.next = 303;
    rejects(&generation);

    generation.header.next = 300;
    generation.receipts[0] = None;
    rejects(&generation);
}

#[test]
fn receipts_require_current_epoch_unique_keys_sequences_and_watermarks() {
    let mut generation = formatted();
    generation.header.epoch = 2;
    generation.header.sequence = 9;
    generation.header.next = 20;
    generation.receipts[0] = Some(direct_record(10, 11, 21, 2, 3, 4, Payload::new(0, 0, &[])));
    assert_eq!(validate(&generation), Ok(()));

    let mut broken = generation.clone();
    broken.header.epoch = 1;
    rejects(&broken);

    let mut broken = generation.clone();
    broken.receipts[1] = Some(direct_record(12, 11, 21, 2, 5, 6, Payload::new(0, 0, &[])));
    rejects(&broken);

    let mut broken = generation.clone();
    broken.receipts[1] = Some(direct_record(12, 13, 22, 2, 2, 4, Payload::new(0, 0, &[])));
    rejects(&broken);

    let mut broken = generation.clone();
    broken.receipts[0] = Some(direct_record(10, 20, 21, 2, 3, 4, Payload::new(0, 0, &[])));
    rejects(&broken);
}

#[test]
fn committed_receipts_for_one_object_form_a_monotonic_history() {
    let mut chained = formatted();
    chained.header.epoch = 2;
    chained.header.sequence = 9;
    chained.header.next = 7;
    chained.nodes[4] = file(5, 1, 1, b"live", 6, Payload::new(0, 0, &[]));
    chained.receipts[0] = Some(direct_record(5, 6, 11, 2, 3, 4, Payload::new(0, 0, &[])));
    chained.receipts[1] = Some(direct_record(5, 6, 12, 2, 4, 6, Payload::new(0, 0, &[])));
    assert_eq!(validate(&chained), Ok(()));

    let mut overlapping = chained.clone();
    overlapping.receipts[1] = Some(direct_record(5, 6, 12, 2, 3, 6, Payload::new(0, 0, &[])));
    assert!(
        overlapping.receipts[0]
            .unwrap()
            .validate(overlapping.header.sequence, overlapping.header.next)
            .is_ok()
    );
    assert!(
        overlapping.receipts[1]
            .unwrap()
            .validate(overlapping.header.sequence, overlapping.header.next)
            .is_ok()
    );
    rejects(&overlapping);

    // This admission was made against version 3 before a competing direct
    // commit advanced the same live file to version 5. Pending work may now be
    // stale; it is not a second successful version transition.
    let mut stale_admission = formatted();
    stale_admission.header.epoch = 2;
    stale_admission.header.sequence = 9;
    stale_admission.header.next = 7;
    stale_admission.nodes[4] = file(5, 1, 1, b"live", 5, Payload::new(0, 0, &[]));
    stale_admission.receipts[0] = Some(record_in_state(
        direct_record(5, 6, 11, 2, 3, 4, Payload::new(0, 0, &[])),
        RecordState::Admitted,
    ));
    stale_admission.receipts[1] = Some(direct_record(5, 6, 12, 2, 3, 5, Payload::new(0, 0, &[])));
    assert_eq!(validate(&stale_admission), Ok(()));

    let mut late_stale_admission = stale_admission.clone();
    let mut late = record_in_state(
        direct_record(5, 6, 13, 2, 3, 4, Payload::new(0, 0, &[])),
        RecordState::Admitted,
    );
    late.instance = 7;
    late.admission_number = 7;
    late_stale_admission.receipts[0] = Some(late);
    assert!(
        late.validate(
            late_stale_admission.header.sequence,
            late_stale_admission.header.next
        )
        .is_ok()
    );
    rejects(&late_stale_admission);
}

#[test]
fn same_object_version_observations_are_monotonic_across_receipt_states() {
    let admitted = |previous, number, retry_key| {
        let mut record = record_in_state(
            direct_record(
                5,
                6,
                retry_key,
                2,
                previous,
                previous + 1,
                Payload::new(0, 0, &[]),
            ),
            RecordState::Admitted,
        );
        record.instance = number;
        record.admission_number = number;
        record
    };
    let cancelled = |previous, number, retry_key| {
        let mut record = record_in_state(
            direct_record(
                5,
                6,
                retry_key,
                2,
                previous,
                previous + 1,
                Payload::new(0, 0, &[]),
            ),
            RecordState::Cancelled,
        );
        record.instance = number;
        record.admission_number = number;
        record.terminal = number + 1;
        record
    };
    let admitted_commit = |previous, admission, committed, retry_key| {
        let mut record = record_in_state(
            direct_record(
                5,
                6,
                retry_key,
                2,
                previous,
                committed,
                Payload::new(0, 0, &[]),
            ),
            RecordState::AdmittedCommitted,
        );
        record.instance = admission;
        record.admission_number = admission;
        record.committed = committed;
        record.terminal = committed;
        record
    };

    let mut committed_after_higher_observation = formatted();
    committed_after_higher_observation.header.epoch = 2;
    committed_after_higher_observation.header.sequence = 9;
    committed_after_higher_observation.header.next = 7;
    committed_after_higher_observation.nodes[4] =
        file(5, 1, 1, b"live", 7, Payload::new(0, 0, &[]));
    committed_after_higher_observation.receipts[0] = Some(admitted(5, 6, 11));
    committed_after_higher_observation.receipts[1] =
        Some(direct_record(5, 6, 12, 2, 3, 7, Payload::new(0, 0, &[])));
    for receipt in committed_after_higher_observation.receipts.iter().flatten() {
        assert!(
            receipt
                .validate(
                    committed_after_higher_observation.header.sequence,
                    committed_after_higher_observation.header.next
                )
                .is_ok()
        );
    }
    rejects(&committed_after_higher_observation);
    let mut reversed = committed_after_higher_observation.clone();
    reversed.receipts.swap(0, 1);
    rejects(&reversed);

    let mut decreasing_admissions = formatted();
    decreasing_admissions.header.epoch = 2;
    decreasing_admissions.header.sequence = 9;
    decreasing_admissions.header.next = 7;
    decreasing_admissions.nodes[4] = file(5, 1, 1, b"live", 9, Payload::new(0, 0, &[]));
    decreasing_admissions.receipts[0] = Some(admitted(5, 6, 11));
    decreasing_admissions.receipts[1] = Some(admitted(3, 8, 12));
    rejects(&decreasing_admissions);
    decreasing_admissions.receipts.swap(0, 1);
    rejects(&decreasing_admissions);

    let mut intermediate_admission = formatted();
    intermediate_admission.header.epoch = 2;
    intermediate_admission.header.sequence = 9;
    intermediate_admission.header.next = 7;
    intermediate_admission.nodes[4] = file(5, 1, 1, b"live", 9, Payload::new(0, 0, &[]));
    intermediate_admission.receipts[0] = Some(admitted_commit(3, 4, 9, 11));
    intermediate_admission.receipts[1] = Some(admitted(5, 6, 12));
    for receipt in intermediate_admission.receipts.iter().flatten() {
        assert!(
            receipt
                .validate(
                    intermediate_admission.header.sequence,
                    intermediate_admission.header.next
                )
                .is_ok()
        );
    }
    rejects(&intermediate_admission);
    intermediate_admission.receipts.swap(0, 1);
    rejects(&intermediate_admission);

    intermediate_admission.receipts[0] = Some(admitted_commit(3, 4, 9, 11));
    intermediate_admission.receipts[1] = Some(admitted(3, 6, 12));
    assert_eq!(validate(&intermediate_admission), Ok(()));

    let mut intermediate_cancelled = formatted();
    intermediate_cancelled.header.epoch = 2;
    intermediate_cancelled.header.sequence = 9;
    intermediate_cancelled.header.next = 7;
    intermediate_cancelled.nodes[4] = file(5, 1, 1, b"live", 9, Payload::new(0, 0, &[]));
    intermediate_cancelled.receipts[0] = Some(admitted_commit(3, 4, 9, 11));
    intermediate_cancelled.receipts[1] = Some(cancelled(5, 6, 12));
    rejects(&intermediate_cancelled);
    intermediate_cancelled.receipts.swap(0, 1);
    rejects(&intermediate_cancelled);

    let mut cancelled_observation = formatted();
    cancelled_observation.header.epoch = 2;
    cancelled_observation.header.sequence = 9;
    cancelled_observation.header.next = 7;
    cancelled_observation.nodes[4] = file(5, 1, 1, b"live", 9, Payload::new(0, 0, &[]));
    cancelled_observation.receipts[0] = Some(cancelled(5, 6, 11));
    cancelled_observation.receipts[1] = Some(admitted(3, 8, 12));
    rejects(&cancelled_observation);
    cancelled_observation.receipts.swap(0, 1);
    rejects(&cancelled_observation);
}

#[test]
fn retained_receipts_check_live_target_history_for_all_states() {
    for state in [
        RecordState::DirectCommitted,
        RecordState::Admitted,
        RecordState::Cancelled,
        RecordState::AdmittedCommitted,
    ] {
        let record = receipt_for_state(state);
        let mut missing_target = with_record(record);
        allocated(&mut missing_target.map, 20, 1);
        assert!(
            record
                .validate(missing_target.header.sequence, missing_target.header.next)
                .is_ok()
        );
        assert_eq!(validate(&missing_target), Ok(()));

        let mut current_target = with_record(record);
        let (version, payload) = if record.committed != 0 {
            (record.committed, Payload::new(512, 77, &[(20, 1)]))
        } else {
            (record.previous, Payload::new(512, 88, &[(10, 1)]))
        };
        current_target.nodes[4] = file(5, 1, 1, b"live", version, payload);
        allocated(&mut current_target.map, 20, 1);
        if record.committed == 0 {
            allocated(&mut current_target.map, 10, 1);
        }
        assert!(current_target.nodes[4].validate().is_ok());
        assert!(
            record
                .validate(current_target.header.sequence, current_target.header.next)
                .is_ok()
        );
        assert_eq!(validate(&current_target), Ok(()));

        let mut old_target = with_record(record);
        let old_version = if record.committed != 0 {
            record.previous
        } else {
            record.previous - 1
        };
        old_target.nodes[4] = file(5, 1, 1, b"old", old_version, Payload::new(0, 0, &[]));
        allocated(&mut old_target.map, 20, 1);
        assert!(old_target.nodes[4].validate().is_ok());
        assert!(
            record
                .validate(old_target.header.sequence, old_target.header.next)
                .is_ok()
        );
        rejects(&old_target);
    }

    for state in [RecordState::DirectCommitted, RecordState::AdmittedCommitted] {
        let record = receipt_for_state(state);
        let mut newer_target = with_record(record);
        newer_target.nodes[4] = file(5, 1, 1, b"newer", 6, Payload::new(512, 88, &[(10, 1)]));
        allocated(&mut newer_target.map, 10, 1);
        allocated(&mut newer_target.map, 20, 1);
        assert!(newer_target.nodes[4].validate().is_ok());
        assert!(
            record
                .validate(newer_target.header.sequence, newer_target.header.next)
                .is_ok()
        );
        assert_eq!(validate(&newer_target), Ok(()));
    }

    for state in [RecordState::Admitted, RecordState::Cancelled] {
        let mut record = receipt_for_state(state);
        record.instance = 6;
        record.admission_number = 6;
        if state == RecordState::Cancelled {
            record.terminal = 7;
        }
        let mut advanced_before_admission = with_record(record);
        advanced_before_admission.nodes[4] = file(5, 1, 1, b"advanced", 5, Payload::new(0, 0, &[]));
        allocated(&mut advanced_before_admission.map, 20, 1);
        assert!(
            record
                .validate(
                    advanced_before_admission.header.sequence,
                    advanced_before_admission.header.next
                )
                .is_ok()
        );
        assert!(advanced_before_admission.nodes[4].validate().is_ok());
        rejects(&advanced_before_admission);
    }

    let mut directory_target = with_record(receipt_for_state(RecordState::Admitted));
    directory_target.nodes[4] = directory(5, 1, 1, b"directory", 3);
    allocated(&mut directory_target.map, 20, 1);
    assert!(directory_target.nodes[4].validate().is_ok());
    rejects(&directory_target);
}

#[test]
fn exact_current_commit_alias_is_allowed_once_and_historical_snapshots_are_owned() {
    let mut generation = formatted();
    generation.header.epoch = 1;
    generation.header.sequence = 4;
    generation.header.next = 7;
    generation.nodes[4] = file(
        5,
        1,
        1,
        b"live",
        4,
        Payload::new(1024, 77, &[(10, 1), (20, 1)]),
    );
    allocated(&mut generation.map, 10, 1);
    allocated(&mut generation.map, 20, 1);
    generation.receipts[0] = Some(direct_record(
        5,
        6,
        11,
        1,
        3,
        4,
        Payload::new(1024, 77, &[(10, 1), (20, 1)]),
    ));
    assert_eq!(validate(&generation), Ok(()));

    let mut duplicated = generation.clone();
    duplicated.receipts[1] = duplicated.receipts[0];
    rejects(&duplicated);

    let mut historical = formatted();
    historical.header.epoch = 1;
    historical.header.sequence = 6;
    historical.header.next = 102;
    historical.nodes[4] = file(5, 1, 1, b"live", 6, Payload::new(512, 88, &[(10, 1)]));
    historical.receipts[0] = Some(direct_record(
        100,
        101,
        11,
        1,
        3,
        4,
        Payload::new(512, 77, &[(20, 1)]),
    ));
    allocated(&mut historical.map, 10, 1);
    allocated(&mut historical.map, 20, 1);
    assert_eq!(validate(&historical), Ok(()));
}

#[test]
fn allocated_map_must_equal_live_and_retained_payload_ownership() {
    let mut generation = formatted();
    generation.header.next = 6;
    generation.nodes[4] = file(5, 1, 1, b"one", 1, Payload::new(512, 123, &[(10, 1)]));
    allocated(&mut generation.map, 10, 1);
    assert_eq!(validate(&generation), Ok(()));

    let mut leak = generation.clone();
    allocated(&mut leak.map, 11, 1);
    rejects(&leak);

    let mut hole = generation.clone();
    hole.map[0] &= !(1 << 10);
    rejects(&hole);

    let mut overlap = generation.clone();
    overlap.nodes[5] = file(6, 1, 1, b"two", 1, Payload::new(512, 124, &[(10, 1)]));
    overlap.header.next = 7;
    rejects(&overlap);

    let mut cross_kind = generation.clone();
    cross_kind.header.epoch = 1;
    cross_kind.header.sequence = 4;
    cross_kind.header.next = 101;
    cross_kind.receipts[0] = Some(direct_record(
        99,
        100,
        11,
        1,
        3,
        4,
        Payload::new(512, 124, &[(10, 1)]),
    ));
    rejects(&cross_kind);

    let mut partial_alias = formatted();
    partial_alias.header.epoch = 1;
    partial_alias.header.sequence = 4;
    partial_alias.header.next = 7;
    partial_alias.nodes[4] = file(
        5,
        1,
        1,
        b"live",
        4,
        Payload::new(1024, 77, &[(10, 1), (20, 1)]),
    );
    partial_alias.receipts[0] = Some(direct_record(
        5,
        6,
        11,
        1,
        3,
        4,
        Payload::new(1024, 77, &[(10, 1), (21, 1)]),
    ));
    allocated(&mut partial_alias.map, 10, 1);
    allocated(&mut partial_alias.map, 20, 1);
    allocated(&mut partial_alias.map, 21, 1);
    rejects(&partial_alias);

    let mut mismatched_commit = formatted();
    mismatched_commit.header.epoch = 1;
    mismatched_commit.header.sequence = 4;
    mismatched_commit.header.next = 7;
    mismatched_commit.nodes[4] = file(5, 1, 1, b"live", 4, Payload::new(512, 77, &[(10, 1)]));
    // The current live file and retained candidate have disjoint sectors; this
    // failure must come from the exact-version content check.
    mismatched_commit.receipts[0] = Some(direct_record(
        5,
        6,
        11,
        1,
        3,
        4,
        Payload::new(512, 78, &[(20, 1)]),
    ));
    allocated(&mut mismatched_commit.map, 10, 1);
    allocated(&mut mismatched_commit.map, 20, 1);
    assert!(mismatched_commit.nodes[4].validate().is_ok());
    assert!(
        mismatched_commit.receipts[0]
            .unwrap()
            .validate(
                mismatched_commit.header.sequence,
                mismatched_commit.header.next
            )
            .is_ok()
    );
    rejects(&mismatched_commit);

    let mut empty_live = formatted();
    empty_live.header.epoch = 1;
    empty_live.header.sequence = 4;
    empty_live.header.next = 7;
    empty_live.nodes[4] = file(5, 1, 1, b"empty", 4, Payload::new(0, 0, &[]));
    empty_live.receipts[0] = Some(direct_record(
        5,
        6,
        11,
        1,
        3,
        4,
        Payload::new(512, 78, &[(20, 1)]),
    ));
    allocated(&mut empty_live.map, 20, 1);
    assert!(empty_live.nodes[4].validate().is_ok());
    assert!(
        empty_live.receipts[0]
            .unwrap()
            .validate(empty_live.header.sequence, empty_live.header.next)
            .is_ok()
    );
    rejects(&empty_live);

    let mut empty_snapshot = formatted();
    empty_snapshot.header.epoch = 1;
    empty_snapshot.header.sequence = 4;
    empty_snapshot.header.next = 7;
    empty_snapshot.nodes[4] = file(5, 1, 1, b"live", 4, Payload::new(512, 77, &[(10, 1)]));
    empty_snapshot.receipts[0] = Some(direct_record(5, 6, 11, 1, 3, 4, Payload::new(0, 0, &[])));
    allocated(&mut empty_snapshot.map, 10, 1);
    assert!(empty_snapshot.nodes[4].validate().is_ok());
    assert!(
        empty_snapshot.receipts[0]
            .unwrap()
            .validate(empty_snapshot.header.sequence, empty_snapshot.header.next)
            .is_ok()
    );
    rejects(&empty_snapshot);

    let mut receipt_overlap = formatted();
    receipt_overlap.header.epoch = 1;
    receipt_overlap.header.sequence = 6;
    receipt_overlap.header.next = 103;
    receipt_overlap.receipts[0] = Some(direct_record(
        100,
        101,
        11,
        1,
        3,
        4,
        Payload::new(512, 1, &[(30, 1)]),
    ));
    receipt_overlap.receipts[1] = Some(direct_record(
        101,
        102,
        12,
        1,
        5,
        6,
        Payload::new(512, 2, &[(30, 1)]),
    ));
    allocated(&mut receipt_overlap.map, 30, 1);
    rejects(&receipt_overlap);
}
