// SPDX-License-Identifier: Apache-2.0
use crate::{State, base, content};
use rustic_fs::{Error, PreventionReason as Reason, Volume};

fn finish<T: Copy>(
    mut write: rustic_fs::Publication<'_, crate::MemoryDisk, T>,
) -> Result<T, Error> {
    for _ in 0..=17 {
        if let Some(result) = write.result() {
            return Ok(result);
        }
        write.advance()?;
    }
    panic!("publication did not settle within its bounded command sequence");
}

#[test]
fn migration_is_explicit_and_never_invents_or_relabels_a_terminal_cause() {
    let (mut disk, request) = base();
    let mut v = Volume::mount(&mut disk).unwrap();
    assert_eq!(
        v.enable_prevention_reasons(&mut disk),
        Err(Error::Unsupported)
    );
    v.enable_admissions(&mut disk).unwrap();
    let admitted = v.admit_replace(&mut disk, 9, 0, request, b"after").unwrap();
    let count = disk.operations;
    assert!(matches!(
        v.prepare_prevention(&mut disk, 9, admitted.id, Reason::Requested),
        Err(Error::Unsupported)
    ));
    assert_eq!(disk.operations, count);
    let old = v.cancel_admission(&mut disk, 9, admitted.id).unwrap();
    assert_eq!(old.prevention, Some(Reason::Unknown));
    v.enable_prevention_reasons(&mut disk).unwrap();
    let mut disk = disk.recover(true);
    let mut v = Volume::mount(&mut disk).unwrap();
    assert_eq!(v.admission_by_id(9, old.id).unwrap().status, old);
    let count = disk.operations;
    assert_eq!(
        finish(
            v.prepare_prevention(&mut disk, 9, old.id, Reason::AuthorityLost)
                .unwrap()
        ),
        Ok(old)
    );
    assert_eq!(disk.operations, count);
    let new = v
        .admit_replace(&mut disk, 10, 0, request, b"after")
        .unwrap();
    let cancelled = v.cancel_admission(&mut disk, 10, new.id).unwrap();
    assert_eq!(cancelled.prevention, Some(Reason::Requested));
    assert_eq!(v.admission_by_id(9, old.id).unwrap().status, old);
    content(&v, &mut disk, request.id, b"before");
}

#[test]
fn every_upgrade_cut_preserves_old_admissions_causes_and_completed_receipts() {
    let (mut base, request) = base();
    let mut v = Volume::mount(&mut base).unwrap();
    v.enable_admissions(&mut base).unwrap();
    let first = v
        .admit_replace(&mut base, 9, 0, request, b"prevented")
        .unwrap();
    let first = v.cancel_admission(&mut base, 9, first.id).unwrap();
    let second = v
        .admit_replace(&mut base, 10, 0, request, b"after")
        .unwrap();
    let receipt = finish(v.prepare_admitted(&mut base, 10, second.id).unwrap()).unwrap();
    let second = v.admission_by_id(10, second.id).unwrap().status;
    for tear in [0, 8, 64, 80, 81, 82, 511, 512] {
        for cut in 0..=28 {
            let mut disk = base.recover(true);
            let mut v = Volume::mount(&mut disk).unwrap();
            disk.fail = Some(cut);
            disk.tear = tear;
            assert_eq!(
                v.enable_prevention_reasons(&mut disk),
                if cut < 28 {
                    Err(Error::Uncertain)
                } else {
                    Ok(())
                }
            );
            for durable in [false, true] {
                let mut disk = disk.recover(durable);
                let mut v = Volume::mount(&mut disk).unwrap();
                assert_eq!(disk.operations, 0, "mount never completes a format upgrade");
                assert_eq!(v.admission_by_id(9, first.id).unwrap().status, first);
                let a = v.admission_by_id(10, second.id).unwrap();
                assert_eq!(a.status, second);
                assert_eq!(a.receipt, Some(receipt));
                assert_eq!(a.bytes, b"after");
                v.enable_prevention_reasons(&mut disk).unwrap();
                assert_eq!((disk.live[8][8], disk.live[13][8]), (5, 5));
                assert_eq!(v.admission_by_id(9, first.id).unwrap().status, first);
                let count = disk.operations;
                v.enable_prevention_reasons(&mut disk).unwrap();
                v.enable_admissions(&mut disk).unwrap();
                assert_eq!(disk.operations, count, "completed upgrades are idempotent");
                content(&v, &mut disk, request.id, b"after");
            }
        }
    }
}

#[test]
fn every_reason_publication_cut_recovers_only_prepared_or_exact_terminal_cause() {
    let (mut base, request) = base();
    let mut v = Volume::mount(&mut base).unwrap();
    v.enable_admissions(&mut base).unwrap();
    let admitted = v.admit_replace(&mut base, 9, 0, request, b"never").unwrap();
    v.enable_prevention_reasons(&mut base).unwrap();
    for reason in [
        Reason::Requested,
        Reason::VersionConflict,
        Reason::AuthorityLost,
    ] {
        for tear in [0, 64, 80, 81, 82, 511, 512] {
            for cut in 0..=14 {
                let mut disk = base.recover(true);
                let mut v = Volume::mount(&mut disk).unwrap();
                disk.fail = Some(cut);
                disk.tear = tear;
                let result = finish(
                    v.prepare_prevention(&mut disk, 9, admitted.id, reason)
                        .unwrap(),
                );
                if cut < 14 {
                    assert_eq!(result, Err(Error::Uncertain));
                    assert!(matches!(
                        v.admission_by_id(9, admitted.id),
                        Err(Error::Uncertain)
                    ));
                } else {
                    assert_eq!(result.unwrap().prevention, Some(reason));
                }
                for durable in [false, true] {
                    let mut disk = disk.recover(durable);
                    let v = Volume::mount(&mut disk).unwrap();
                    let a = v.admission_by_id(9, admitted.id).unwrap();
                    match a.status.state {
                        State::Admitted => assert_eq!(a.status, admitted),
                        State::Cancelled => assert_eq!(a.status.prevention, Some(reason)),
                        State::Committed => panic!("prevention must not publish file data"),
                    }
                    assert_eq!(a.receipt, None);
                    assert_eq!(a.bytes, b"never");
                    assert_eq!(disk.operations, 0);
                    content(&v, &mut disk, request.id, b"before");
                }
            }
        }
    }
}

#[test]
fn valid_checksums_cannot_hide_unknown_codes_or_causes_on_nonterminal_records() {
    let (mut disk, request) = base();
    let mut v = Volume::mount(&mut disk).unwrap();
    v.enable_admissions(&mut disk).unwrap();
    v.enable_prevention_reasons(&mut disk).unwrap();
    let first = v.admit_replace(&mut disk, 9, 0, request, b"first").unwrap();
    v.admit_replace(&mut disk, 10, 0, request, b"second")
        .unwrap();
    v.cancel_admission(&mut disk, 9, first.id).unwrap();
    for (offset, value) in [(81, 4), (81, 255), (82, 1), (511, 1), (1536 + 81, 1)] {
        let mut bad = super::media::rewrite(&disk, |r| r[512 + offset] = value);
        assert!(matches!(Volume::mount(&mut bad), Err(Error::Corrupt)));
    }
    let mut bad = super::media::rewrite(&disk, |r| r[..8].copy_from_slice(b"RUSTREC3"));
    assert!(matches!(Volume::mount(&mut bad), Err(Error::Corrupt)));
}

#[test]
fn unreadable_fallback_bank_fences_the_upgrade_until_explicit_recovery() {
    struct ReadFailure(crate::MemoryDisk);
    impl rustic_fs::Disk for ReadFailure {
        fn read(&mut self, _: u64, _: &mut [u8; 512]) -> Result<(), Error> {
            Err(Error::Io)
        }
        fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
            rustic_fs::Disk::write(&mut self.0, sector, bytes)
        }
        fn flush(&mut self) -> Result<(), Error> {
            rustic_fs::Disk::flush(&mut self.0)
        }
    }
    let (mut disk, _) = base();
    let mut v = Volume::mount(&mut disk).unwrap();
    v.enable_admissions(&mut disk).unwrap();
    let mut disk = ReadFailure(disk.recover(true));
    assert_eq!(
        v.enable_prevention_reasons(&mut disk),
        Err(Error::Uncertain)
    );
    assert_eq!(disk.0.operations, 14);
    assert_eq!(v.prevention_reasons_enabled(), Err(Error::Uncertain));
    let mut disk = disk.0.recover(true);
    let mut recovered = Volume::mount(&mut disk).unwrap();
    assert_eq!(disk.operations, 0);
    recovered.enable_prevention_reasons(&mut disk).unwrap();
    assert_eq!((disk.live[8][8], disk.live[13][8]), (5, 5));
}
