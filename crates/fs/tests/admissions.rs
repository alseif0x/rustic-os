// SPDX-License-Identifier: Apache-2.0
mod support;
mod admissions {
    mod cuts;
    mod media;
    mod polling;
    mod reasons;
    mod validation;
}
use rustic_fs::{
    AdmissionState as State, Error, Kind, PublicationCancel as Cancel, PublicationPhase as Phase,
    Replacement, Retry, Volume,
};
use support::MemoryDisk;

fn base() -> (MemoryDisk, Replacement) {
    let mut disk = MemoryDisk::new();
    let mut v = Volume::initialize(&mut disk).unwrap();
    let file = v.create(&mut disk, 4, b"file", Kind::File).unwrap();
    let file = v
        .replace(&mut disk, file.id, file.version, b"before")
        .unwrap();
    v.enable_recovery(&mut disk, [7; 16]).unwrap();
    v.enable_operations(&mut disk).unwrap();
    (
        disk,
        Replacement {
            workspace: 4,
            retry: Retry {
                lineage: [7; 16],
                epoch: 1,
                key: 42,
            },
            id: file.id,
            version: file.version,
        },
    )
}

fn content(v: &Volume, disk: &mut MemoryDisk, id: u32, expected: &[u8]) {
    let mut bytes = [0; 1024];
    let count = v.read(disk, id, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..count], expected);
}

#[test]
fn bounded_admission_inventory_excludes_legacy_receipts_without_reclaiming_their_slots() {
    let (mut disk, request) = base();
    let mut v = Volume::mount(&mut disk).unwrap();
    v.replace_scoped(&mut disk, 9, 0, request, b"legacy")
        .unwrap();
    v.enable_admissions(&mut disk).unwrap();
    let next = Replacement {
        version: v.stat(request.id).unwrap().version,
        retry: Retry {
            key: 43,
            ..request.retry
        },
        ..request
    };
    let admitted = v.admit_replace(&mut disk, 10, 0, next, b"next").unwrap();
    let operations = disk.operations;
    assert!(v.retained_admission(0).unwrap().is_none());
    let (subject, view) = v.retained_admission(1).unwrap().unwrap();
    assert_eq!(subject, 10);
    assert_eq!(view.status, admitted);
    assert_eq!(view.bytes, b"next");
    assert!(matches!(
        v.retained_admission(rustic_fs::RETAINED),
        Err(Error::Invalid)
    ));
    assert_eq!(
        disk.operations, operations,
        "enumeration must never issue I/O"
    );
    let third = Replacement {
        retry: Retry {
            key: 44,
            ..request.retry
        },
        ..next
    };
    assert_eq!(
        v.admit_replace(&mut disk, 10, 0, third, b"third"),
        Err(Error::Full)
    );
}

#[test]
fn admission_and_cancellation_survive_restart_and_never_reexecute_on_retry() {
    let (mut disk, request) = base();
    let mut v = Volume::mount(&mut disk).unwrap();
    v.enable_admissions(&mut disk).unwrap();
    let accepted = v.admit_replace(&mut disk, 9, 0, request, b"after").unwrap();
    assert_eq!(accepted.state, State::Admitted);
    assert_eq!(accepted.id.number, v.sequence());
    assert!(accepted.id.number > request.version);
    assert_eq!(v.stat(request.id).unwrap().version, request.version);
    let mut disk = disk.recover(true);
    let mut v = Volume::mount(&mut disk).unwrap();
    let count = disk.operations;
    assert_eq!(v.admission_by_id(9, accepted.id).unwrap().status, accepted);
    assert_eq!(
        v.admit_replace(&mut disk, 9, 0, request, b"after").unwrap(),
        accepted
    );
    assert!(matches!(
        v.operation_by_retry(9, 4, request.retry),
        Err(Error::Busy)
    ));
    assert_eq!(v.advance_epoch(&mut disk), Err(Error::Busy));
    assert_eq!(disk.operations, count);
    for boundary in 0..=15 {
        let mut write = v.prepare_admitted(&mut disk, 9, accepted.id).unwrap();
        for _ in 0..boundary {
            write.advance().unwrap();
        }
        assert_eq!(write.cancel(), Ok(Cancel::Cancelled));
        drop(write);
        assert_eq!(v.admission_by_id(9, accepted.id).unwrap().status, accepted);
        content(&v, &mut disk, request.id, b"before");
    }
    let cancelled = v.cancel_admission(&mut disk, 9, accepted.id).unwrap();
    assert_eq!(cancelled.state, State::Cancelled);
    assert!(cancelled.terminal > accepted.id.number);
    let mut disk = disk.recover(true);
    let mut v = Volume::mount(&mut disk).unwrap();
    v.remove(&mut disk, request.id).unwrap();
    let count = disk.operations;
    assert_eq!(
        v.admit_replace(&mut disk, 9, 0, request, b"after").unwrap(),
        cancelled
    );
    assert_eq!(
        v.cancel_admission(&mut disk, 9, accepted.id).unwrap(),
        cancelled
    );
    assert!(matches!(
        v.prepare_admitted(&mut disk, 9, accepted.id),
        Err(Error::Cancelled)
    ));
    assert_eq!(
        v.admit_replace(&mut disk, 9, 0, request, b"changed"),
        Err(Error::IdempotencyConflict)
    );
    assert!(matches!(
        v.admission_by_id(8, accepted.id),
        Err(Error::OutcomeUnknown)
    ));
    assert_eq!(disk.operations, count);
}

#[test]
fn effect_and_receipt_settle_together_under_the_original_admission_identity() {
    let (mut disk, request) = base();
    let mut v = Volume::mount(&mut disk).unwrap();
    v.enable_admissions(&mut disk).unwrap();
    let accepted = v.admit_replace(&mut disk, 9, 0, request, b"after").unwrap();
    let receipt;
    {
        let mut write = v.prepare_admitted(&mut disk, 9, accepted.id).unwrap();
        for _ in 0..16 {
            write.advance().unwrap();
        }
        assert_eq!(write.cancel(), Ok(Cancel::TooLate));
        assert_eq!(write.result(), None);
        assert_eq!(write.advance(), Ok(Phase::Committed));
        receipt = write.result().unwrap();
    }
    let mut disk = disk.recover(true);
    let mut v = Volume::mount(&mut disk).unwrap();
    let old = v.admission_by_id(9, accepted.id).unwrap();
    assert_eq!(old.status.state, State::Committed);
    assert_eq!(old.receipt, Some(receipt));
    assert_eq!(old.instance, accepted.id.number);
    assert!(receipt.committed > accepted.id.number);
    let settled = old.status;
    content(&v, &mut disk, request.id, b"after");
    v.remove(&mut disk, request.id).unwrap();
    let count = disk.operations;
    assert_eq!(
        v.cancel_admission(&mut disk, 9, accepted.id).unwrap(),
        settled
    );
    let mut replay = v.prepare_admitted(&mut disk, 9, accepted.id).unwrap();
    assert_eq!(replay.advance(), Ok(Phase::Committed));
    assert_eq!(replay.result(), Some(receipt));
    drop(replay);
    assert_eq!(
        v.operation_by_id(9, [7; 16], receipt.committed)
            .unwrap()
            .receipt,
        receipt
    );
    assert_eq!(disk.operations, count);
}

#[test]
fn admission_reserves_capacity_but_not_versions_authority_or_permission_to_forget() {
    let (mut disk, request) = base();
    let mut v = Volume::mount(&mut disk).unwrap();
    v.enable_admissions(&mut disk).unwrap();
    let accepted = v.admit_replace(&mut disk, 9, 0, request, b"after").unwrap();
    let second = v
        .admit_replace(&mut disk, 10, 0, request, b"other")
        .unwrap();
    let count = disk.operations;
    assert_eq!(
        v.admit_replace(&mut disk, 11, 0, request, b"full"),
        Err(Error::Full)
    );
    assert_eq!(v.advance_epoch(&mut disk), Err(Error::Busy));
    assert_eq!(
        v.cancel_admission(&mut disk, 10, accepted.id),
        Err(Error::OutcomeUnknown)
    );
    assert_eq!(disk.operations, count);
    v.replace(&mut disk, request.id, request.version, b"human edit")
        .unwrap();
    assert!(matches!(
        v.prepare_admitted(&mut disk, 9, accepted.id),
        Err(Error::Version)
    ));
    v.cancel_admission(&mut disk, 9, accepted.id).unwrap();
    assert_eq!(v.advance_epoch(&mut disk), Err(Error::Busy));
    v.cancel_admission(&mut disk, 10, second.id).unwrap();
    assert_eq!(v.advance_epoch(&mut disk), Ok(2));
    assert!(matches!(
        v.admission_by_id(9, accepted.id),
        Err(Error::OutcomeUnknown)
    ));
    assert!(matches!(
        v.admission_by_retry(9, 4, request.retry),
        Err(Error::ExpiredEpoch)
    ));
    let later = Replacement {
        version: v.stat(request.id).unwrap().version,
        retry: Retry {
            epoch: 2,
            ..request.retry
        },
        ..request
    };
    let new = v.admit_replace(&mut disk, 9, 0, later, b"new").unwrap();
    assert!(new.id.number > second.id.number);
}
