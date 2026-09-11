// SPDX-License-Identifier: Apache-2.0
use crate::{State, base, content};
use rustic_fs::{Error, PublicationPhase, Volume};

#[test]
fn every_terminal_retention_cut_keeps_cancellation_or_fences_the_old_key() {
    let (mut base, request) = base();
    let mut v = Volume::mount(&mut base).unwrap();
    v.enable_admissions(&mut base).unwrap();
    let a = v
        .admit_replace(&mut base, 9, 0, request, b"cancelled")
        .unwrap();
    let cancelled = v.cancel_admission(&mut base, 9, a.id).unwrap();
    for tear in [0, 64, 80, 511, 512] {
        for cut in 0..=14 {
            let mut disk = base.recover(true);
            let mut v = Volume::mount(&mut disk).unwrap();
            disk.fail = Some(cut);
            disk.tear = tear;
            assert_eq!(
                v.advance_epoch(&mut disk),
                if cut < 14 {
                    Err(Error::Uncertain)
                } else {
                    Ok(2)
                }
            );
            for durable in [false, true] {
                let mut disk = disk.recover(durable);
                let mut v = Volume::mount(&mut disk).unwrap();
                let count = disk.operations;
                let retry = v.admit_replace(&mut disk, 9, 0, request, b"cancelled");
                assert!(retry == Ok(cancelled) || retry == Err(Error::ExpiredEpoch));
                assert_eq!(disk.operations, count);
                content(&v, &mut disk, request.id, b"before");
            }
        }
    }
}

#[test]
fn all_admission_cancellation_and_effect_cuts_recover_only_legal_states() {
    let (mut base, request) = base();
    Volume::mount(&mut base)
        .unwrap()
        .enable_admissions(&mut base)
        .unwrap();
    for transition in 0..3 {
        let mut start = base.recover(true);
        let mut v = Volume::mount(&mut start).unwrap();
        if transition != 0 {
            v.admit_replace(&mut start, 9, 0, request, &[0xa5; 1024])
                .unwrap();
        }
        let steps = if transition == 2 { 17 } else { 14 };
        for tear in [0, 1, 64, 72, 80, 81, 256, 511, 512] {
            for cut in 0..=steps {
                let mut disk = start.recover(true);
                let mut v = Volume::mount(&mut disk).unwrap();
                disk.fail = Some(cut);
                disk.tear = tear;
                let result = if transition == 0 {
                    v.admit_replace(&mut disk, 9, 0, request, &[0xa5; 1024])
                        .map(|_| ())
                } else {
                    let id = v.admission_by_retry(9, 4, request.retry).unwrap().status.id;
                    if transition == 1 {
                        v.cancel_admission(&mut disk, 9, id).map(|_| ())
                    } else {
                        let mut write = v.prepare_admitted(&mut disk, 9, id).unwrap();
                        let mut result = Ok(());
                        while write.phase() != PublicationPhase::Committed {
                            if let Err(e) = write.advance() {
                                result = Err(e);
                                break;
                            }
                        }
                        result
                    }
                };
                assert_eq!(
                    result,
                    if cut < steps {
                        Err(Error::Uncertain)
                    } else {
                        Ok(())
                    }
                );
                if cut < steps {
                    assert!(matches!(v.stat(request.id), Err(Error::Uncertain)));
                }
                for durable in [false, true] {
                    let mut recovered = disk.recover(durable);
                    let v = Volume::mount(&mut recovered).unwrap();
                    match v.admission_by_retry(9, 4, request.retry) {
                        Ok(a) => {
                            assert_eq!(a.bytes, &[0xa5; 1024]);
                            let committed = a.status.state == State::Committed;
                            assert_eq!(a.receipt.is_some(), committed);
                            if transition == 0 {
                                assert_eq!(a.status.state, State::Admitted);
                            }
                            if transition == 1 {
                                assert_ne!(a.status.state, State::Committed);
                            }
                            if transition == 2 {
                                assert_ne!(a.status.state, State::Cancelled);
                            }
                            assert_eq!(
                                v.stat(request.id).unwrap().version,
                                a.receipt.map_or(request.version, |r| r.committed)
                            );
                            content(
                                &v,
                                &mut recovered,
                                request.id,
                                if committed { &[0xa5; 1024] } else { b"before" },
                            );
                        }
                        Err(Error::OutcomeUnknown) if transition == 0 => {
                            content(&v, &mut recovered, request.id, b"before")
                        }
                        other => panic!(
                            "transition {transition} cut {cut} tear {tear} durable {durable}: {other:?}"
                        ),
                    }
                }
            }
        }
    }
}

#[test]
fn all_v4_upgrade_cuts_preserve_v2_and_v3_receipts_without_inventing_admissions() {
    let (mut base, request) = base();
    let mut v = Volume::mount(&mut base).unwrap();
    let legacy = v
        .replace_tracked(
            &mut base,
            9,
            request.retry,
            request.id,
            request.version,
            b"legacy",
        )
        .unwrap();
    let scoped = v
        .replace_scoped(
            &mut base,
            9,
            0,
            rustic_fs::Replacement {
                version: legacy.committed,
                ..request
            },
            b"scoped",
        )
        .unwrap();
    for tear in [0, 1, 64, 80, 81, 256, 511, 512] {
        for cut in 0..=28 {
            let mut disk = base.recover(true);
            let mut v = Volume::mount(&mut disk).unwrap();
            disk.fail = Some(cut);
            disk.tear = tear;
            assert_eq!(
                v.enable_admissions(&mut disk),
                if cut < 28 {
                    Err(Error::Uncertain)
                } else {
                    Ok(())
                }
            );
            for durable in [false, true] {
                let mut disk = disk.recover(durable);
                let mut v = Volume::mount(&mut disk).unwrap();
                v.enable_admissions(&mut disk).unwrap();
                assert_eq!(disk.live[8][8], 4);
                assert_eq!(disk.live[13][8], 4);
                assert_eq!(v.receipt(9, request.retry).unwrap(), legacy);
                assert_eq!(
                    v.operation_by_retry(9, 4, request.retry).unwrap().receipt,
                    scoped
                );
                assert!(matches!(
                    v.admission_by_retry(9, 4, request.retry),
                    Err(Error::Unsupported)
                ));
                let count = disk.operations;
                assert_eq!(
                    v.admit_replace(&mut disk, 9, 0, request, b"before"),
                    Err(Error::Unsupported)
                );
                assert_eq!(disk.operations, count);
            }
        }
    }
}
