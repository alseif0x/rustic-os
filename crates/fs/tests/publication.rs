// SPDX-License-Identifier: Apache-2.0
mod support;
use rustic_fs::{
    Error, Kind, PublicationCancel as Cancel, PublicationPhase as Phase, Replacement, Retry, Volume,
};
use support::MemoryDisk;

fn base() -> (MemoryDisk, Replacement) {
    let mut disk = MemoryDisk::new();
    let mut volume = Volume::initialize(&mut disk).unwrap();
    volume.enable_recovery(&mut disk, [7; 16]).unwrap();
    volume.enable_operations(&mut disk).unwrap();
    let file = volume.create(&mut disk, 4, b"file", Kind::File).unwrap();
    let file = volume
        .replace(&mut disk, file.id, file.version, b"before")
        .unwrap();
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

fn content(volume: &Volume, disk: &mut MemoryDisk, id: u32) -> Vec<u8> {
    let mut bytes = [0; 1024];
    let count = volume.read(disk, id, 0, &mut bytes).unwrap();
    bytes[..count].to_vec()
}

#[test]
fn every_prepublication_boundary_cancels_without_a_receipt_or_live_effect() {
    let (base, request) = base();
    for boundary in 0..=15 {
        let mut disk = base.recover(true);
        let mut volume = Volume::mount(&mut disk).unwrap();
        let sequence = volume.sequence();
        let start = disk.operations;
        {
            let mut write = volume
                .prepare_scoped(&mut disk, 9, 0, request, b"after")
                .unwrap();
            assert_eq!(write.phase(), Phase::Preparing);
            for _ in 0..boundary {
                write.advance().unwrap();
            }
            assert_eq!(write.result(), None);
            if boundary == 15 {
                assert_eq!(write.phase(), Phase::ReadyToPublish);
            }
            assert_eq!(write.cancel(), Ok(Cancel::Cancelled));
            assert_eq!(write.cancel(), Ok(Cancel::Cancelled));
            assert_eq!(write.advance(), Ok(Phase::Cancelled));
        }
        assert_eq!(disk.operations - start, boundary);
        assert_eq!(volume.sequence(), sequence);
        assert_eq!(volume.stat(request.id).unwrap().version, request.version);
        assert_eq!(content(&volume, &mut disk, request.id), b"before");
        assert!(matches!(
            volume.operation_by_retry(9, 4, request.retry),
            Err(Error::OutcomeUnknown)
        ));
        for durable in [false, true] {
            let mut recovered = disk.recover(durable);
            let mounted = Volume::mount(&mut recovered).unwrap();
            assert_eq!(content(&mounted, &mut recovered, request.id), b"before");
            assert!(matches!(
                mounted.operation_by_retry(9, 4, request.retry),
                Err(Error::OutcomeUnknown)
            ));
        }
        // A volatile cancellation reserves neither a retry key nor a durable ID.
        volume
            .replace_scoped(&mut disk, 9, 0, request, b"after")
            .unwrap();
        assert_eq!(content(&volume, &mut disk, request.id), b"after");
    }
}

#[test]
fn late_cancellation_cannot_skip_settlement_or_change_a_committed_receipt() {
    let (base, request) = base();
    for boundary in [16, 17] {
        let mut disk = base.recover(true);
        let mut volume = Volume::mount(&mut disk).unwrap();
        let start = disk.operations;
        let receipt;
        {
            let mut write = volume
                .prepare_scoped(&mut disk, 9, 0, request, b"after")
                .unwrap();
            for _ in 0..boundary {
                write.advance().unwrap();
            }
            assert_eq!(write.cancel(), Ok(Cancel::TooLate));
            if boundary == 16 {
                assert_eq!(write.phase(), Phase::Settling);
                assert_eq!(write.result(), None);
                assert_eq!(write.advance(), Ok(Phase::Committed));
            }
            receipt = write.result().unwrap();
            assert_eq!(write.advance(), Ok(Phase::Committed));
            assert_eq!(write.cancel(), Ok(Cancel::TooLate));
            assert_eq!(write.result(), Some(receipt));
        }
        assert_eq!(disk.operations - start, 17);
        for durable in [false, true] {
            let mut recovered = disk.recover(durable);
            let mounted = Volume::mount(&mut recovered).unwrap();
            assert_eq!(content(&mounted, &mut recovered, request.id), b"after");
            assert_eq!(
                mounted
                    .operation_by_retry(9, 4, request.retry)
                    .unwrap()
                    .receipt,
                receipt
            );
        }
    }
}

#[test]
fn drop_releases_only_known_safe_writers_and_forgetting_cannot_bypass_fencing() {
    let (base, request) = base();
    for forget in [false, true] {
        for boundary in 0..=17 {
            let mut disk = base.recover(true);
            let mut volume = Volume::mount(&mut disk).unwrap();
            let mut write = volume
                .prepare_scoped(&mut disk, 9, 0, request, b"after")
                .unwrap();
            for _ in 0..boundary {
                write.advance().unwrap();
            }
            if forget {
                core::mem::forget(write);
            } else {
                drop(write);
            }
            let fenced = boundary != 17 && (forget || boundary == 16);
            assert_eq!(volume.stat(request.id).is_err(), fenced);
            if fenced {
                assert!(matches!(
                    volume.replace(&mut disk, request.id, request.version, b"unsafe"),
                    Err(Error::Uncertain)
                ));
            }
        }
    }
}

#[test]
fn failed_commands_cannot_be_relabelled_as_cancelled_or_retried_in_place() {
    let (base, request) = base();
    for cut in 0..17 {
        for tear in [0, 256, 512] {
            let mut disk = base.recover(true);
            let mut volume = Volume::mount(&mut disk).unwrap();
            disk.operations = 0;
            disk.fail = Some(cut);
            disk.tear = tear;
            {
                let mut write = volume
                    .prepare_scoped(&mut disk, 9, 0, request, b"after")
                    .unwrap();
                for _ in 0..cut {
                    write.advance().unwrap();
                }
                assert_eq!(write.advance(), Err(Error::Uncertain));
                assert_eq!(write.phase(), Phase::Uncertain);
                assert_eq!(write.cancel(), Err(Error::Uncertain));
                assert_eq!(write.advance(), Err(Error::Uncertain));
                assert_eq!(write.result(), None);
            }
            assert_eq!(disk.operations, cut + 1);
            assert!(matches!(volume.stat(request.id), Err(Error::Uncertain)));
            for durable in [false, true] {
                let mut recovered = disk.recover(durable);
                let mounted = Volume::mount(&mut recovered).unwrap();
                let bytes = content(&mounted, &mut recovered, request.id);
                assert!(bytes == b"before" || bytes == b"after");
                assert_eq!(
                    mounted.operation_by_retry(9, 4, request.retry).is_ok(),
                    bytes == b"after"
                );
            }
        }
    }
}

#[test]
fn historical_replay_is_terminal_and_issues_no_disk_commands() {
    let (mut disk, request) = base();
    let mut volume = Volume::mount(&mut disk).unwrap();
    let receipt = volume
        .replace_scoped(&mut disk, 9, 0, request, b"after")
        .unwrap();
    volume.remove(&mut disk, request.id).unwrap();
    let start = disk.operations;
    {
        let mut write = volume
            .prepare_scoped(&mut disk, 9, 0, request, b"after")
            .unwrap();
        assert_eq!(write.result(), Some(receipt));
        assert_eq!(write.cancel(), Ok(Cancel::TooLate));
        assert_eq!(write.advance(), Ok(Phase::Committed));
    }
    assert_eq!(disk.operations, start);
}

#[test]
fn legacy_volume_and_preparation_failures_keep_the_same_publication_boundary() {
    let mut base = MemoryDisk::new();
    let mut volume = Volume::initialize(&mut base).unwrap();
    let file = volume.create(&mut base, 4, b"file", Kind::File).unwrap();
    for boundary in 0..=10 {
        let mut disk = base.recover(true);
        let mut volume = Volume::mount(&mut disk).unwrap();
        let start = disk.operations;
        assert!(matches!(
            volume.prepare_replace(&mut disk, file.id, 0, b"bad"),
            Err(Error::Version)
        ));
        assert_eq!(disk.operations, start);
        let mut write = volume
            .prepare_replace(&mut disk, file.id, file.version, b"after")
            .unwrap();
        for _ in 0..boundary {
            write.advance().unwrap();
        }
        if boundary <= 8 {
            assert_eq!(write.cancel(), Ok(Cancel::Cancelled));
        } else {
            assert_eq!(write.cancel(), Ok(Cancel::TooLate));
            write.advance().unwrap();
            assert!(write.result().is_some());
        }
        drop(write);
        assert_eq!(
            volume.stat(file.id).unwrap().length,
            if boundary <= 8 { 0 } else { 5 }
        );
    }
}

#[test]
fn a_disk_panic_after_submission_does_not_release_the_writer() {
    struct PanickingDisk(MemoryDisk);
    impl rustic_fs::Disk for PanickingDisk {
        fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
            self.0.read(sector, bytes)
        }
        fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
            self.0.write(sector, bytes)?;
            panic!("simulated adapter unwind after submission")
        }
        fn flush(&mut self) -> Result<(), Error> {
            self.0.flush()
        }
    }
    let (mut disk, request) = base();
    let mut volume = Volume::mount(&mut disk).unwrap();
    let mut disk = PanickingDisk(disk);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut write = volume
            .prepare_scoped(&mut disk, 9, 0, request, b"after")
            .unwrap();
        write.advance().unwrap();
    }));
    assert!(outcome.is_err());
    assert!(matches!(volume.stat(request.id), Err(Error::Uncertain)));
}
