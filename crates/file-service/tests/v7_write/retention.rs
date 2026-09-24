// SPDX-License-Identifier: Apache-2.0
//! Owner retention maintenance through the V7 service: refused while a
//! transfer is open, then reclaiming every terminal record, freeing only the
//! snapshot sectors no live file owns and publishing the next retry epoch, so
//! old-epoch retries and lookups expire and repeated writes can continue.
use super::lookup::{by_id, by_retry, remount};
use super::*;
use rustic_abi::files::MAINTAIN_RETENTION;
use rustic_file_service::Maintenance7;

const SIZE: usize = 1500;

/// Commit `count` writes of [`SIZE`] bytes with keys from `first`, each
/// replacing the previous version, and return their receipts.
fn fill(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    writer: &Writer,
    (workspace, file): (u32, u32),
    first: u64,
    count: u64,
) -> Vec<Operation> {
    let mut receipts = Vec::new();
    for key in first..first + count {
        let current = version(server.volume(), file);
        let request = replacement(server.volume(), workspace, file, current, key);
        let (receipt, _) = writer
            .replace(server, disk, request, &pattern(key as u8, SIZE))
            .unwrap();
        receipts.push(receipt);
    }
    receipts
}

/// The request an exact retry of `receipt` presents.
fn retry_of(receipt: &Operation) -> operation::Replacement {
    operation::Replacement {
        workspace: receipt.workspace,
        resource: receipt.resource,
        expected_version: receipt.previous_version,
        retry: receipt.retry,
    }
}

/// Send the chunks of `bytes` from byte `from` on.
fn chunks_from(
    writer: &Writer,
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    object: u32,
    bytes: &[u8],
    from: usize,
) -> Result<(), Error> {
    for (index, chunk) in bytes[from..].chunks(DATA).enumerate() {
        let mut p = Packet::new(REPLACE_CHUNK);
        p.id = object;
        p.arg = (from + index * DATA) as u32;
        p.count = chunk.len() as u8;
        p.data[..chunk.len()].copy_from_slice(chunk);
        status(writer.send(server, disk, p))?;
    }
    Ok(())
}

fn records(volume: &Volume7) -> usize {
    volume.retained_records().unwrap().iter().flatten().count()
}

/// Payload sectors held only by retained snapshots: those maintenance frees.
fn snapshot_only_sectors(volume: &Volume7) -> u64 {
    let live: Vec<_> = volume
        .nodes()
        .unwrap()
        .iter()
        .filter(|node| node.kind == Kind::File)
        .flat_map(|node| node.runs().to_vec())
        .collect();
    volume
        .retained_records()
        .unwrap()
        .iter()
        .flatten()
        .flat_map(|record| record.runs().to_vec())
        .filter(|run| !live.contains(run))
        .map(|run| run.sectors)
        .sum()
}

#[test]
fn maintenance_is_busy_while_a_transfer_is_open_and_changes_nothing() {
    let mut f = fixture();
    let target = (f.workspace, f.file);
    let mut server = Server7::new(&mut f.volume);
    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    let receipts = fill(&mut server, &mut f.disk, &writer, target, 1, 8);
    let last = *receipts.last().unwrap();
    let bytes = pattern(8, SIZE);
    let current = version(server.volume(), f.file);
    let beyond = replacement(server.volume(), f.workspace, f.file, current, 9);
    assert_eq!(
        writer.open(&mut server, &mut f.disk, beyond, 1),
        Err(Error::Full)
    );

    // At Full only an exact retry can hold a transfer open: its verifying
    // stage reserves nothing.
    writer
        .open(&mut server, &mut f.disk, retry_of(&last), SIZE)
        .unwrap();
    writer
        .chunks(&mut server, &mut f.disk, f.file, &bytes[..600])
        .unwrap();
    let header = *server.volume().header().unwrap();
    let io = (f.disk.writes, f.disk.flushes);
    assert_eq!(server.maintain_retention(&mut f.disk), Err(Error::Busy));
    assert_eq!(*server.volume().header().unwrap(), header);
    assert_eq!(records(server.volume()), 8);
    assert_eq!((f.disk.writes, f.disk.flushes), io);

    // The refusal left the held retry intact: it completes and replays.
    chunks_from(&writer, &mut server, &mut f.disk, f.file, &bytes, 600).unwrap();
    let (replayed, _) = writer.commit(&mut server, &mut f.disk, f.file).unwrap();
    assert_eq!(replayed, last);

    // With the retry resolved, maintenance succeeds.
    let done = server.maintain_retention(&mut f.disk).unwrap();
    assert_eq!(done.records, 8);
    // A fresh transfer on another slot blocks it the same way until aborted.
    let other = Writer::grant(&mut server, 1, f.workspace, TRACKED_WRITE7, SUBJECT);
    let request = replacement(server.volume(), f.workspace, f.file, current, 1);
    other.open(&mut server, &mut f.disk, request, SIZE).unwrap();
    let header = *server.volume().header().unwrap();
    assert_eq!(server.maintain_retention(&mut f.disk), Err(Error::Busy));
    assert_eq!(*server.volume().header().unwrap(), header);
    other.abort(&mut server, &mut f.disk, f.file).unwrap();
    assert_eq!(
        server.maintain_retention(&mut f.disk).unwrap().epoch,
        header.epoch + 1
    );
}

#[test]
fn maintenance_reclaims_records_frees_snapshot_sectors_and_advances_the_epoch() {
    let mut f = fixture();
    let target = (f.workspace, f.file);
    let mut server = Server7::new(&mut f.volume);
    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    fill(&mut server, &mut f.disk, &writer, target, 1, 8);
    let before = *server.volume().header().unwrap();
    let free = server.volume().free_sectors().unwrap();
    let freed = snapshot_only_sectors(server.volume());
    // Seven superseded 1500-byte snapshots of three sectors; the eighth is the
    // live file and stays allocated.
    assert_eq!(freed, 7 * 3);

    let done = server.maintain_retention(&mut f.disk).unwrap();
    assert_eq!(
        done,
        Maintenance7 {
            previous_epoch: before.epoch,
            epoch: before.epoch + 1,
            records: 8,
            sectors: freed as u32,
        }
    );
    assert_eq!(server.volume().free_sectors().unwrap(), free + freed);
    assert_eq!(records(server.volume()), 0);
    assert_eq!(
        server.volume().header().unwrap().sequence,
        before.sequence + 1
    );
    drop(server);
    assert_eq!(read_all(&f.volume, &mut f.disk, f.file), pattern(8, SIZE));

    // The published state is what a cold mount selects.
    let volume = remount(&mut f.disk);
    assert_eq!(volume.header().unwrap().epoch, before.epoch + 1);
    assert_eq!(records(&volume), 0);
    assert_eq!(volume.free_sectors().unwrap(), free + freed);

    // Nothing left to reclaim still advances the epoch, freeing nothing.
    let mut volume = volume;
    let mut server = Server7::new(&mut volume);
    let again = server.maintain_retention(&mut f.disk).unwrap();
    assert_eq!(
        (
            again.previous_epoch,
            again.epoch,
            again.records,
            again.sectors
        ),
        (before.epoch + 1, before.epoch + 2, 0, 0)
    );
}

#[test]
fn old_epoch_retries_and_lookups_expire_and_repeated_writes_use_the_new_epoch() {
    let mut f = fixture();
    let target = (f.workspace, f.file);
    let mut server = Server7::new(&mut f.volume);
    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    let mut epoch = server.volume().header().unwrap().epoch;
    let mut key = 1;
    for cycle in 0..3 {
        let receipts = fill(&mut server, &mut f.disk, &writer, target, key, 8);
        key += 8;
        let current = version(server.volume(), f.file);
        let beyond = replacement(server.volume(), f.workspace, f.file, current, key);
        assert_eq!(
            writer.open(&mut server, &mut f.disk, beyond, 1),
            Err(Error::Full),
            "cycle {cycle}"
        );
        let first = receipts[0];
        // This slot caches the receipt it looked up last.
        writer
            .lookup(&mut server, &mut f.disk, by_id(&first))
            .unwrap();

        let done = server.maintain_retention(&mut f.disk).unwrap();
        assert_eq!((done.previous_epoch, done.epoch), (epoch, epoch + 1));
        epoch += 1;

        // The cached receipt went with its record.
        assert_eq!(
            writer
                .part(&mut server, &mut f.disk, first.id, 40)
                .unwrap_err(),
            Error::OutcomeUnknown
        );
        assert_eq!(
            writer
                .lookup(&mut server, &mut f.disk, by_retry(&first))
                .unwrap_err(),
            Error::ExpiredEpoch
        );
        assert_eq!(
            writer
                .lookup(&mut server, &mut f.disk, by_id(&first))
                .unwrap_err(),
            Error::OutcomeUnknown
        );
        // An exact retry of an old-epoch write expires before any other
        // check; so does a fresh write that still names the old epoch.
        assert_eq!(
            writer.open(&mut server, &mut f.disk, retry_of(&first), SIZE),
            Err(Error::ExpiredEpoch)
        );
        let mut stale = replacement(server.volume(), f.workspace, f.file, current, key);
        stale.retry.epoch = first.retry.epoch;
        assert_eq!(
            writer.open(&mut server, &mut f.disk, stale, 1),
            Err(Error::ExpiredEpoch)
        );
        assert_eq!(server.volume().open_stages(), 0);

        // The same key is fresh in the new epoch, and useful work continues.
        let mut request = retry_of(&first);
        request.expected_version = Version::new(current).unwrap();
        request.retry.epoch = Epoch::new(epoch).unwrap();
        let (receipt, _) = writer
            .replace(&mut server, &mut f.disk, request, &pattern(200, 700))
            .unwrap();
        assert_eq!(receipt.retry.epoch.value(), epoch);
        assert_eq!(receipt.previous_version.value(), current);
        assert_eq!(records(server.volume()), 1);
        // Only one record exists, so the next cycle fills seven more.
        let (next, _) = writer
            .lookup(&mut server, &mut f.disk, by_retry(&receipt))
            .unwrap();
        assert_eq!(next, receipt);
        let refill = fill(&mut server, &mut f.disk, &writer, target, key, 7);
        key += 7;
        assert_eq!(refill.len(), 7);
        server.maintain_retention(&mut f.disk).unwrap();
        epoch += 1;
    }
    assert_eq!(server.volume().header().unwrap().epoch, epoch);
}

#[test]
fn a_client_packet_cannot_request_maintenance() {
    let mut f = fixture();
    let target = (f.workspace, f.file);
    let mut server = Server7::new(&mut f.volume);
    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    fill(&mut server, &mut f.disk, &writer, target, 1, 8);
    let header = *server.volume().header().unwrap();
    let io = (f.disk.writes, f.disk.flushes);
    let reply = writer.send(&mut server, &mut f.disk, Packet::new(MAINTAIN_RETENTION));
    assert_ne!(reply.status, 0);
    assert_eq!(*server.volume().header().unwrap(), header);
    assert_eq!(records(server.volume()), 8);
    assert_eq!((f.disk.writes, f.disk.flushes), io);
}

#[test]
fn a_failed_publication_fences_the_volume_and_keeps_the_old_generation() {
    let mut f = fixture();
    let target = (f.workspace, f.file);
    let mut server = Server7::new(&mut f.volume);
    let writer = Writer::grant(&mut server, 0, f.workspace, TRACKED_WRITE7, SUBJECT);
    let receipts = fill(&mut server, &mut f.disk, &writer, target, 1, 3);
    let cached = receipts.last().unwrap().id;
    writer.part(&mut server, &mut f.disk, cached, 40).unwrap();
    let header = *server.volume().header().unwrap();
    let current = version(server.volume(), f.file);
    let next = replacement(server.volume(), f.workspace, f.file, current, 50);
    f.disk.fail_writes_from = Some(f.disk.writes);
    // The owner cannot be told the epoch stayed: the outcome is uncertain,
    // and the volume is fenced until a remount, so later requests are too.
    assert_eq!(
        server.maintain_retention(&mut f.disk),
        Err(Error::Uncertain)
    );
    assert_eq!(server.volume().header(), Err(FsError::Uncertain));
    assert_eq!(
        writer.open(&mut server, &mut f.disk, next, 1),
        Err(Error::Uncertain)
    );
    assert_eq!(
        server.maintain_retention(&mut f.disk),
        Err(Error::Uncertain)
    );
    // A fenced volume keeps no cached receipt that may name reclaimed records.
    assert_eq!(
        writer
            .part(&mut server, &mut f.disk, cached, 40)
            .unwrap_err(),
        Error::OutcomeUnknown
    );
    drop(server);
    f.disk.fail_writes_from = None;
    // No header was written: a cold mount still selects the old generation.
    let volume = remount(&mut f.disk);
    assert_eq!(*volume.header().unwrap(), header);
    assert_eq!(records(&volume), 3);
}
