// SPDX-License-Identifier: Apache-2.0
mod support;
use rustic_fs::{Error, Kind, Retry, Volume};
use support::MemoryDisk;
fn base() -> (MemoryDisk, u32, u64, Retry) {
    let mut d = MemoryDisk::new();
    let mut v = Volume::initialize(&mut d).unwrap();
    let f = v.create(&mut d, 4, b"atomic", Kind::File).unwrap();
    let f = v.replace(&mut d, f.id, f.version, b"before").unwrap();
    v.enable_recovery(&mut d, [7; 16]).unwrap();
    (
        d,
        f.id,
        f.version,
        Retry {
            lineage: [7; 16],
            epoch: 1,
            key: 42,
        },
    )
}
#[test]
fn retained_arguments_replay_after_later_edit_delete_and_reboot_without_writes() {
    let (mut d, id, version, key) = base();
    let mut v = Volume::mount(&mut d).unwrap();
    let receipt = v
        .replace_tracked(&mut d, 9, key, id, version, b"after")
        .unwrap();
    v.replace(&mut d, id, receipt.committed, b"human edit")
        .unwrap();
    let mut v = Volume::mount(&mut d).unwrap();
    let n = d.operations;
    assert_eq!(
        v.replace_tracked(&mut d, 9, key, id, version, b"after"),
        Ok(receipt)
    );
    assert_eq!(
        v.replace_tracked(&mut d, 9, key, id, version, b"other"),
        Err(Error::IdempotencyConflict)
    );
    assert_eq!(v.receipt(8, key), Err(Error::OutcomeUnknown));
    assert_eq!(d.operations, n);
    v.remove(&mut d, id).unwrap();
    assert_eq!(v.receipt(9, key), Ok(receipt));
    let mut wrong = key;
    wrong.lineage = [8; 16];
    assert_eq!(v.receipt(9, wrong), Err(Error::Lineage));
}
#[test]
fn full_receipts_reject_before_data_then_rotation_fences_old_keys_durably() {
    let (mut d, id, version, key) = base();
    let mut v = Volume::mount(&mut d).unwrap();
    let a = v
        .replace_tracked(&mut d, 9, key, id, version, b"a")
        .unwrap();
    let b = v
        .replace_tracked(&mut d, 9, Retry { key: 43, ..key }, id, a.committed, b"b")
        .unwrap();
    let n = d.operations;
    assert_eq!(
        v.replace_tracked(&mut d, 9, Retry { key: 44, ..key }, id, b.committed, b"c"),
        Err(Error::Full)
    );
    assert_eq!(d.operations, n);
    assert_eq!(v.advance_epoch(&mut d), Ok(2));
    let mut v = Volume::mount(&mut d).unwrap();
    assert_eq!(v.receipt(9, key), Err(Error::ExpiredEpoch));
    assert_eq!(
        v.replace_tracked(&mut d, 9, key, id, b.committed, b"c"),
        Err(Error::ExpiredEpoch)
    );
    assert!(
        v.replace_tracked(&mut d, 9, Retry { epoch: 2, ..key }, id, b.committed, b"c")
            .is_ok()
    );
}
#[test]
fn every_replacement_cut_recovers_effect_and_receipt_together() {
    let (base, id, version, key) = base();
    let mut probe = base.recover(true);
    let mut v = Volume::mount(&mut probe).unwrap();
    let start = probe.operations;
    v.replace_tracked(&mut probe, 9, key, id, version, &[0x72; 1024])
        .unwrap();
    let count = probe.operations - start;
    assert_eq!(count, 17);
    for tear in [0, 1, 16, 32, 256, 511, 512] {
        for cut in 0..=count {
            let mut d = base.recover(true);
            let mut v = Volume::mount(&mut d).unwrap();
            d.fail = Some(cut);
            d.tear = tear;
            let result = v.replace_tracked(&mut d, 9, key, id, version, &[0x72; 1024]);
            assert_eq!(result.is_ok(), cut == count);
            if result.is_err() {
                assert_eq!(v.receipt(9, key), Err(Error::Uncertain));
            }
            for durable in [false, true] {
                let mut recovered = d.recover(durable);
                let mut mounted = Volume::mount(&mut recovered).unwrap();
                let mut bytes = [0; 1024];
                let length = mounted.read(&mut recovered, id, 0, &mut bytes).unwrap();
                match mounted.receipt(9, key) {
                    Ok(r) => {
                        assert_eq!(length, 1024);
                        assert_eq!(bytes, [0x72; 1024]);
                        assert_eq!(mounted.stat(id).unwrap().version, r.committed);
                        let n = recovered.operations;
                        assert_eq!(
                            mounted.replace_tracked(
                                &mut recovered,
                                9,
                                key,
                                id,
                                version,
                                &[0x72; 1024]
                            ),
                            Ok(r)
                        );
                        assert_eq!(recovered.operations, n);
                    }
                    Err(Error::OutcomeUnknown) => {
                        assert_eq!(&bytes[..length], b"before");
                        assert_eq!(mounted.stat(id).unwrap().version, version);
                        assert!(result.is_err());
                    }
                    other => panic!("cut={cut} tear={tear} durable={durable}: {other:?}"),
                }
            }
        }
    }
}
#[test]
fn every_retention_cut_keeps_evidence_or_rejects_old_epoch() {
    let (mut base, id, version, key) = base();
    let mut v = Volume::mount(&mut base).unwrap();
    let receipt = v
        .replace_tracked(&mut base, 9, key, id, version, b"after")
        .unwrap();
    for cut in 0..=14 {
        for tear in [0, 32, 511, 512] {
            let mut d = base.recover(true);
            let mut v = Volume::mount(&mut d).unwrap();
            d.fail = Some(cut);
            d.tear = tear;
            let _ = v.advance_epoch(&mut d);
            for durable in [false, true] {
                let mut d = d.recover(durable);
                let mut v = Volume::mount(&mut d).unwrap();
                let n = d.operations;
                let replay = v.replace_tracked(&mut d, 9, key, id, version, b"after");
                assert!(replay == Ok(receipt) || replay == Err(Error::ExpiredEpoch));
                assert_eq!(d.operations, n);
            }
        }
    }
}

#[test]
fn interrupted_legacy_upgrade_resumes_without_changing_files_or_identity() {
    let mut base = MemoryDisk::new();
    let mut v = Volume::initialize(&mut base).unwrap();
    let f = v.create(&mut base, 4, b"preserve", Kind::File).unwrap();
    let f = v
        .replace(&mut base, f.id, f.version, b"owner data")
        .unwrap();
    for cut in 0..14 {
        for tear in [0, 1, 16, 32, 256, 511, 512] {
            let mut d = base.recover(true);
            let mut v = Volume::mount(&mut d).unwrap();
            d.fail = Some(cut);
            d.tear = tear;
            let _ = v.enable_recovery(&mut d, [7; 16]);
            for durable in [false, true] {
                let mut d = d.recover(durable);
                let mut v = Volume::mount(&mut d).unwrap();
                v.enable_recovery(&mut d, [7; 16]).unwrap();
                let mut bytes = [0; 32];
                let n = v.read(&mut d, f.id, 0, &mut bytes).unwrap();
                assert_eq!(&bytes[..n], b"owner data");
                assert_eq!(v.stat(f.id).unwrap().version, f.version);
                assert_eq!(v.recovery_info(), Ok(([7; 16], 1)));
                assert_eq!(v.enable_recovery(&mut d, [8; 16]), Err(Error::Lineage));
            }
        }
    }
}
