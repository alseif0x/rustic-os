// SPDX-License-Identifier: Apache-2.0
mod support;
use rustic_fs::{Error, Kind, Replacement, Retry, Volume};
use support::MemoryDisk;
fn base() -> (MemoryDisk, Replacement) {
    let mut disk = MemoryDisk::new();
    let mut volume = Volume::initialize(&mut disk).unwrap();
    let file = volume.create(&mut disk, 4, b"file", Kind::File).unwrap();
    let file = volume
        .replace(&mut disk, file.id, file.version, b"before")
        .unwrap();
    volume.enable_recovery(&mut disk, [7; 16]).unwrap();
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
#[test]
fn namespaces_and_historical_lookup_do_not_alias_legacy_or_foreign_receipts() {
    let (mut disk, request) = base();
    let mut volume = Volume::mount(&mut disk).unwrap();
    assert_eq!(
        volume.replace_scoped(&mut disk, 9, 0, request, b"after"),
        Err(Error::Unsupported)
    );
    let legacy = volume
        .replace_tracked(
            &mut disk,
            9,
            request.retry,
            request.id,
            request.version,
            b"legacy",
        )
        .unwrap();
    volume.enable_operations(&mut disk).unwrap();
    let request = Replacement {
        version: legacy.committed,
        ..request
    };
    let scoped = volume
        .replace_scoped(&mut disk, 9, 0, request, b"scoped")
        .unwrap();
    assert_eq!(volume.receipt(9, request.retry).unwrap(), legacy);
    volume.remove(&mut disk, request.id).unwrap();
    let mut volume = Volume::mount(&mut disk).unwrap();
    let start = disk.operations;
    assert_eq!(
        volume
            .replace_scoped(&mut disk, 9, 0, request, b"scoped")
            .unwrap(),
        scoped
    );
    assert_eq!(
        volume.replace_scoped(&mut disk, 9, 0, request, b"changed"),
        Err(Error::IdempotencyConflict)
    );
    let old = volume
        .operation_by_id(9, [7; 16], scoped.committed)
        .unwrap();
    assert_eq!(old.bytes, b"scoped");
    assert_eq!(old.instance, scoped.committed);
    assert_eq!(old.workspace, 4);
    assert!(matches!(
        volume.operation_by_id(8, [7; 16], scoped.committed),
        Err(Error::OutcomeUnknown)
    ));
    assert!(matches!(
        volume.operation_by_id(9, [7; 16], legacy.committed),
        Err(Error::OutcomeUnknown)
    ));
    assert!(matches!(
        volume.operation_by_retry(9, 3, request.retry),
        Err(Error::OutcomeUnknown)
    ));
    assert_eq!(disk.operations, start);
}
#[test]
fn identical_keys_in_distinct_workspaces_commit_independently_and_rotation_fences_both() {
    let (mut disk, a) = base();
    let mut volume = Volume::mount(&mut disk).unwrap();
    let folder = volume
        .create(&mut disk, 4, b"workspace", Kind::Directory)
        .unwrap();
    let file = volume
        .create(&mut disk, folder.id, b"file", Kind::File)
        .unwrap();
    let b = Replacement {
        workspace: folder.id,
        id: file.id,
        version: file.version,
        ..a
    };
    volume.enable_operations(&mut disk).unwrap();
    let ra = volume.replace_scoped(&mut disk, 9, 0, a, b"one").unwrap();
    let rb = volume
        .replace_scoped(&mut disk, 9, ra.committed, b, b"two")
        .unwrap();
    assert_ne!(ra.committed, rb.committed);
    assert_eq!(
        volume
            .operation_by_retry(9, b.workspace, b.retry)
            .unwrap()
            .instance,
        ra.committed
    );
    let count = disk.operations;
    assert_eq!(
        volume.replace_scoped(&mut disk, 10, 0, b, b"other"),
        Err(Error::Full)
    );
    assert_eq!(disk.operations, count);
    volume.advance_epoch(&mut disk).unwrap();
    let mut volume = Volume::mount(&mut disk).unwrap();
    assert!(matches!(
        volume.operation_by_retry(9, a.workspace, a.retry),
        Err(Error::ExpiredEpoch)
    ));
    assert!(matches!(
        volume.operation_by_id(9, [7; 16], rb.committed),
        Err(Error::OutcomeUnknown)
    ));
    let b = Replacement {
        version: rb.committed,
        retry: Retry {
            epoch: 2,
            ..b.retry
        },
        ..b
    };
    let later = volume
        .replace_scoped(&mut disk, 9, 0, b, b"rebooted")
        .unwrap();
    assert!(
        volume
            .operation_by_id(9, [7; 16], later.committed)
            .unwrap()
            .instance
            > ra.committed
    );
}
#[test]
fn every_migration_cut_preserves_legacy_receipts_and_never_invents_scoped_identity() {
    let (mut base, request) = base();
    let mut volume = Volume::mount(&mut base).unwrap();
    let old = volume
        .replace_tracked(
            &mut base,
            9,
            request.retry,
            request.id,
            request.version,
            b"old",
        )
        .unwrap();
    for tear in [0, 1, 48, 64, 256, 511, 512] {
        for cut in 0..=28 {
            let mut disk = base.recover(true);
            let mut volume = Volume::mount(&mut disk).unwrap();
            disk.fail = Some(cut);
            disk.tear = tear;
            let result = volume.enable_operations(&mut disk);
            if cut < 28 {
                assert_eq!(result, Err(Error::Uncertain));
            } else {
                result.unwrap();
            }
            for durable in [false, true] {
                let mut recovered = disk.recover(durable);
                let mut mounted = Volume::mount(&mut recovered).unwrap();
                assert_eq!(mounted.receipt(9, request.retry).unwrap(), old);
                mounted.enable_operations(&mut recovered).unwrap();
                assert_eq!(recovered.live[8][8], 3);
                assert_eq!(recovered.live[13][8], 3);
                assert!(matches!(
                    mounted.operation_by_retry(9, 4, request.retry),
                    Err(Error::OutcomeUnknown)
                ));
            }
        }
    }
}
#[test]
fn every_scoped_effect_cut_recovers_the_original_arguments_and_identity_atomically() {
    let (mut base, request) = base();
    Volume::mount(&mut base)
        .unwrap()
        .enable_operations(&mut base)
        .unwrap();
    for tear in [0, 1, 48, 64, 256, 511, 512] {
        for cut in 0..=17 {
            let mut disk = base.recover(true);
            let mut volume = Volume::mount(&mut disk).unwrap();
            disk.fail = Some(cut);
            disk.tear = tear;
            let result = volume.replace_scoped(&mut disk, 9, 0, request, &[0xa5; 1024]);
            if cut < 17 {
                assert_eq!(result, Err(Error::Uncertain));
            } else {
                result.unwrap();
            }
            for durable in [false, true] {
                let mut recovered = disk.recover(durable);
                let mounted = Volume::mount(&mut recovered).unwrap();
                match mounted.operation_by_retry(9, 4, request.retry) {
                    Ok(old) => {
                        assert_eq!(old.bytes, &[0xa5; 1024]);
                        assert_eq!(old.instance, old.receipt.committed);
                        assert_eq!(
                            mounted.stat(request.id).unwrap().version,
                            old.receipt.committed
                        );
                    }
                    Err(Error::OutcomeUnknown) => {
                        assert_eq!(mounted.stat(request.id).unwrap().version, request.version)
                    }
                    other => panic!("unexpected recovery {other:?}"),
                }
            }
        }
    }
}
