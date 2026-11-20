// SPDX-License-Identifier: Apache-2.0
//! The streamed tracked publication shares the existing V7 barriers, while
//! retaining the selected generation until the header and final flush settle.
use super::*;
use rustic_fs::Publication7Cancel;

#[test]
fn pollable_tracked_commit_matches_blocking_media_and_replays_without_io() {
    for size in [0, 1, 513, 4096] {
        let bytes = pattern(19, size);
        let mut blocking = seed_one_file();
        let mut first = mount(&mut blocking);
        let expected = first
            .replace_tracked(&mut blocking, identity(90), 1, &bytes)
            .unwrap();
        let mut disk = seed_one_file();
        let mut volume = mount(&mut disk);
        let mut stage = stream(
            &mut volume,
            &mut disk,
            identity(90),
            1,
            &bytes,
            Stage7Kind::Tracked,
        )
        .unwrap();
        let mut ready = Ready(&mut disk);
        let mut publication = volume.finish_tracked_poll(&mut ready, &mut stage).unwrap();
        assert_eq!(publication.result(), None);
        assert_eq!(settle(&mut publication), Ok(expected));
        drop(publication);
        assert_same_media(&disk, &blocking, "pollable tracked");
        assert_eq!(volume.abort_stage(stage), Err(Error::Invalid));
        let mut retry = stream(
            &mut volume,
            &mut disk,
            identity(90),
            1,
            &bytes,
            Stage7Kind::Tracked,
        )
        .unwrap();
        let operations = disk.operations;
        let mut ready = Ready(&mut disk);
        let publication = volume.finish_tracked_poll(&mut ready, &mut retry).unwrap();
        assert_eq!(publication.phase(), Publication7Phase::Committed);
        assert_eq!(publication.result(), Some(expected));
        drop(publication);
        assert_eq!(disk.operations, operations);
    }
}

#[test]
fn stopping_before_header_discards_the_candidate_but_late_stop_settles() {
    for commands in [0, 1, 50, 101, 102, 103] {
        let mut disk = seed_one_file();
        let mut volume = mount(&mut disk);
        let bytes = pattern(20, 900);
        let mut stage = stream(
            &mut volume,
            &mut disk,
            identity(91),
            1,
            &bytes,
            Stage7Kind::Tracked,
        )
        .unwrap();
        let mut ready = Ready(&mut disk);
        let mut publication = volume.finish_tracked_poll(&mut ready, &mut stage).unwrap();
        for _ in 0..commands {
            assert!(!matches!(publication.poll_advance(), Poll::Ready(Err(_))));
        }
        let late = commands >= 102;
        assert_eq!(
            publication.abort_before_header(),
            Ok(if late {
                Publication7Cancel::TooLate
            } else {
                Publication7Cancel::Cancelled
            })
        );
        if late {
            assert_eq!(settle(&mut publication).unwrap().committed, 2);
        } else {
            assert_eq!(publication.result(), None);
        }
        drop(publication);
        assert_eq!(volume.open_stages(), 0);
        assert_eq!(volume.header().unwrap().sequence, if late { 2 } else { 1 });
        let (mut durable, recovered) = remount(&disk);
        let node = recovered.stat(5).unwrap();
        assert_eq!(
            read_snapshot(&mut durable, node.runs(), node.length),
            if late { bytes } else { BEFORE.to_vec() }
        );
        assert_eq!(
            recovered
                .retained_records()
                .unwrap()
                .iter()
                .flatten()
                .count(),
            usize::from(late)
        );
    }
}

#[test]
fn every_pollable_tracked_publication_failure_fences_and_remounts_the_old_head() {
    for cut in 0..103 {
        let mut disk = seed_one_file();
        let mut volume = mount(&mut disk);
        let free = volume.free_sectors().unwrap();
        let mut stage = stream(
            &mut volume,
            &mut disk,
            identity(92),
            1,
            &[21; 512],
            Stage7Kind::Tracked,
        )
        .unwrap();
        disk.operations = 0;
        disk.fail_at = Some(cut);
        let mut ready = Ready(&mut disk);
        let mut publication = volume.finish_tracked_poll(&mut ready, &mut stage).unwrap();
        assert_eq!(settle(&mut publication), Err(Error::Uncertain), "cut {cut}");
        drop(publication);
        assert_eq!(volume.header().err(), Some(Error::Uncertain));
        let (_, recovered) = remount(&disk);
        assert_eq!(recovered.header().unwrap().sequence, 1, "cut {cut}");
        assert_eq!(recovered.free_sectors(), Ok(free));
        assert!(
            recovered
                .retained_records()
                .unwrap()
                .iter()
                .all(Option::is_none)
        );
    }
}

/// Hold exactly one command for one poll, without retaining the caller buffer.
struct Held<'a> {
    disk: &'a mut Sparse,
    at: usize,
    completed: usize,
    pending: bool,
}

impl Held<'_> {
    fn hold(&mut self) -> bool {
        if self.completed == self.at && !self.pending {
            self.pending = true;
            true
        } else {
            self.pending = false;
            self.completed += 1;
            false
        }
    }
}

impl PollDisk for Held<'_> {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        if self.hold() {
            Poll::Pending
        } else {
            Poll::Ready(self.disk.write(sector, bytes))
        }
    }
    fn poll_flush(&mut self) -> Poll<Result<(), Error>> {
        if self.hold() {
            Poll::Pending
        } else {
            Poll::Ready(self.disk.flush())
        }
    }
}

impl PollDisk7 for Held<'_> {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), Error>> {
        if self.hold() {
            Poll::Pending
        } else {
            Poll::Ready(self.disk.read(sector, bytes))
        }
    }
}

#[test]
fn held_command_drains_before_stop_and_header_or_final_flush_cannot_be_stopped() {
    for at in [0, 100, 101, 102] {
        let mut disk = seed_one_file();
        let mut volume = mount(&mut disk);
        let mut stage = stream(
            &mut volume,
            &mut disk,
            identity(93),
            1,
            &[22; 900],
            Stage7Kind::Tracked,
        )
        .unwrap();
        let mut held = Held {
            disk: &mut disk,
            at,
            completed: 0,
            pending: false,
        };
        let mut publication = volume.finish_tracked_poll(&mut held, &mut stage).unwrap();
        while !publication.pending() {
            assert!(!matches!(publication.poll_advance(), Poll::Ready(Err(_))));
        }
        let late = at >= 101;
        assert_eq!(
            publication.abort_before_header(),
            Ok(if late {
                Publication7Cancel::TooLate
            } else {
                Publication7Cancel::Draining
            })
        );
        if late {
            assert_eq!(settle(&mut publication).unwrap().committed, 2);
        } else {
            assert_eq!(
                publication.poll_advance(),
                Poll::Ready(Ok(Publication7Phase::Cancelled))
            );
            assert_eq!(publication.result(), None);
        }
        drop(publication);
        assert!(!held.pending);
        assert_eq!(volume.header().unwrap().sequence, if late { 2 } else { 1 });
        let (_, recovered) = remount(&disk);
        assert_eq!(
            recovered.header().unwrap().sequence,
            if late { 2 } else { 1 }
        );
    }
}

#[test]
fn dropping_with_an_outstanding_tracked_command_fences_until_remount() {
    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);
    let mut stage = stream(
        &mut volume,
        &mut disk,
        identity(94),
        1,
        &[23; 512],
        Stage7Kind::Tracked,
    )
    .unwrap();
    let mut held = Held {
        disk: &mut disk,
        at: 0,
        completed: 0,
        pending: false,
    };
    let mut publication = volume.finish_tracked_poll(&mut held, &mut stage).unwrap();
    assert_eq!(publication.poll_advance(), Poll::Pending);
    drop(publication);
    assert!(held.pending);
    assert_eq!(volume.header().err(), Some(Error::Uncertain));
    assert_eq!(remount(&disk).1.header().unwrap().sequence, 1);
}

#[test]
fn pollable_commit_preserves_retained_snapshot_and_other_open_stage() {
    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);
    let retained = volume
        .replace_tracked(&mut disk, identity(95), 1, &[24; 600])
        .unwrap();
    let other = volume.create(&mut disk, 4, b"other", Kind::File).unwrap();
    let mut first = stream(
        &mut volume,
        &mut disk,
        identity(96),
        retained.committed,
        &[25; 900],
        Stage7Kind::Tracked,
    )
    .unwrap();
    let mut second = stream(
        &mut volume,
        &mut disk,
        WriteIdentity7 {
            object: other.id,
            ..identity(97)
        },
        other.version,
        &[26; 700],
        Stage7Kind::Tracked,
    )
    .unwrap();
    let mut ready = Ready(&mut disk);
    let mut publication = volume.finish_tracked_poll(&mut ready, &mut first).unwrap();
    settle(&mut publication).unwrap();
    drop(publication);
    assert_eq!(volume.open_stages(), 1);
    let second_record = volume.finish_tracked(&mut disk, &mut second).unwrap();
    assert_eq!(
        read_snapshot(&mut disk, retained.runs(), retained.length),
        [24; 600]
    );
    assert_eq!(
        read_snapshot(&mut disk, second_record.runs(), second_record.length),
        [26; 700]
    );
    let (_, recovered) = remount(&disk);
    assert_eq!(
        recovered
            .retained_records()
            .unwrap()
            .iter()
            .flatten()
            .count(),
        3
    );
}

/// The final flush completes durably, but its completion status is an error.
struct DurableError<'a>(&'a mut Sparse, usize);

impl PollDisk for DurableError<'_> {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        Poll::Ready(self.0.write(sector, bytes))
    }
    fn poll_flush(&mut self) -> Poll<Result<(), Error>> {
        self.0.flush().unwrap();
        self.1 += 1;
        Poll::Ready(if self.1 == 2 { Err(Error::Io) } else { Ok(()) })
    }
}

impl PollDisk7 for DurableError<'_> {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), Error>> {
        Poll::Ready(self.0.read(sector, bytes))
    }
}

#[test]
fn final_flush_error_can_recover_the_committed_record_after_remount() {
    let mut disk = seed_one_file();
    let mut volume = mount(&mut disk);
    let mut stage = stream(
        &mut volume,
        &mut disk,
        identity(98),
        1,
        &[27; 600],
        Stage7Kind::Tracked,
    )
    .unwrap();
    let mut failed = DurableError(&mut disk, 0);
    let mut publication = volume.finish_tracked_poll(&mut failed, &mut stage).unwrap();
    assert_eq!(settle(&mut publication), Err(Error::Uncertain));
    drop(publication);
    assert_eq!(volume.header().err(), Some(Error::Uncertain));
    let (mut durable, recovered) = remount(&disk);
    let record = recovered.retained_records().unwrap()[0].unwrap();
    assert_eq!(
        (record.state, record.committed, record.retry_key),
        (RecordState::DirectCommitted, 2, 98)
    );
    assert_eq!(
        read_snapshot(&mut durable, record.runs(), record.length),
        [27; 600]
    );
    assert_eq!(recovered.stat(5).unwrap().version, record.committed);
}
