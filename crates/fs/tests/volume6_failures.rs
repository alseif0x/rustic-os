// SPDX-License-Identifier: Apache-2.0
//! Failure injection for the v6 commit discipline (#51).
//!
//! One commit is a payload write, the inactive generation's structures, a
//! flush, the publication header and a second flush. These tests fail each of
//! those operations in turn and require that
//!
//! * the generation the header named before the commit is still the one a later
//!   mount reads, so a torn commit cannot take the published volume with it;
//! * the failure is `Uncertain`, and the mount writes, replays and reads nothing
//!   further until it is mounted again from durable storage;
//! * the header is never written before the payload and the inactive generation
//!   it names have been flushed, which is what keeps a device that persists the
//!   header early from leaving a volume whose header names a generation that
//!   never landed.

use std::collections::BTreeMap;

use rustic_fs::{
    DATA_SECTORS, Disk, Error, Kind, MAX_FILE_V6, Node6, OBJECTS_V6, Retry, Volume6, map_sector,
    mount6, provision6,
};

/// The v6 layout's header sector (`format6::HEADER_SECTOR`, not re-exported).
const HEADER_SECTOR: u64 = 8;
const LINEAGE: [u8; 16] = [7; 16];
const RETRY_KEY: u64 = 11;

/// A disk that keeps sector writes in a live cache and makes them durable on a
/// successful flush, so a test can recover from a chosen point. `early_header`
/// models a device that persists the header sector the moment it is written,
/// whatever order the commit issued its writes in.
#[derive(Clone)]
struct Journal {
    live: BTreeMap<u64, [u8; 512]>,
    durable: BTreeMap<u64, [u8; 512]>,
    operations: usize,
    fail_at: Option<usize>,
    early_header: bool,
}

impl Journal {
    fn new() -> Self {
        Self {
            live: BTreeMap::new(),
            durable: BTreeMap::new(),
            operations: 0,
            fail_at: None,
            early_header: false,
        }
    }
    /// The state a reboot finds: everything a successful flush made durable, and
    /// no armed failure, because an injected failure belongs to a test run and
    /// not to the device.
    fn recover(&self) -> Self {
        Self {
            live: self.durable.clone(),
            durable: self.durable.clone(),
            operations: 0,
            fail_at: None,
            early_header: self.early_header,
        }
    }
    fn corrupt(&mut self, sector: u64, offset: usize) {
        self.live.entry(sector).or_insert([0; 512])[offset] ^= 1;
    }
    fn step(&mut self) -> bool {
        let fail = self.fail_at == Some(self.operations);
        self.operations += 1;
        fail
    }
}

impl Disk for Journal {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        *bytes = self.live.get(&sector).copied().unwrap_or([0; 512]);
        Ok(())
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        if self.step() {
            return Err(Error::Io);
        }
        self.live.insert(sector, *bytes);
        if self.early_header && sector == HEADER_SECTOR {
            self.durable.insert(sector, *bytes);
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), Error> {
        if self.step() {
            return Err(Error::Io);
        }
        self.durable = self.live.clone();
        Ok(())
    }
}

fn retry(key: u64) -> Retry {
    Retry {
        lineage: LINEAGE,
        epoch: 1,
        key,
    }
}

/// A durable volume holding the file identity 5 at slot 4 with the published
/// bytes `first` and version 2, so a commit under test expects version 2.
fn published_first(early_header: bool) -> Journal {
    let mut disk = Journal::new();
    disk.early_header = early_header;
    let mut volume = provision6(&mut disk, LINEAGE).expect("provision");
    let mut record = Node6::EMPTY;
    record.id = 5;
    record.parent = 4;
    record.kind = Kind::File;
    record.version = 1;
    record.name[..4].copy_from_slice(b"file");
    record.name_length = 4;
    volume.nodes[4] = record;
    volume
        .write_file(&mut disk, 4, 1, b"first")
        .expect("publish the first bytes");
    disk.recover()
}

/// The published bytes of the node with identity `id`.
fn bytes_of(disk: &mut Journal, volume: &Volume6, id: u32) -> Vec<u8> {
    let node = *volume.node(id).expect("file node");
    let mut bytes = vec![0; node.length as usize];
    assert_eq!(
        volume.read_file(disk, &node, &mut bytes),
        Ok(bytes.len()),
        "the record's extent list must still hold its bytes"
    );
    bytes
}

/// The sector writes and flushes one tracked write issues from the fixture, so
/// a test can fail each of them in turn.
fn tracked_steps(base: &Journal) -> usize {
    let mut disk = base.recover();
    let mut volume = mount6(&mut disk).expect("mount");
    disk.operations = 0;
    volume
        .write_tracked(&mut disk, 4, 2, retry(RETRY_KEY), b"second")
        .expect("tracked write");
    disk.operations
}

#[test]
fn every_failed_operation_keeps_the_published_generation_and_fences_the_mount() {
    let base = published_first(false);
    let steps = tracked_steps(&base);
    assert!(steps > 3, "the commit must issue recoverable operations");

    for step in 0..steps {
        let mut disk = base.recover();
        let mut volume = mount6(&mut disk).expect("mount the published generation");
        // Mounting flushes the device (the recovery boundary), so the failure is
        // armed against the commit alone.
        disk.operations = 0;
        disk.fail_at = Some(step);

        // The failure is reported honestly: the operation may have reached the
        // device, so its outcome is unknown rather than an error the caller can
        // retry against this mount.
        assert_eq!(
            volume.write_tracked(&mut disk, 4, 2, retry(RETRY_KEY), b"second"),
            Err(Error::Uncertain),
            "operation {step} must be Uncertain"
        );

        // Nothing else is written or answered, and a replay invents no receipt
        // from the state the failed commit left in memory.
        let operations = disk.operations;
        assert_eq!(
            volume.write_tracked(&mut disk, 4, 2, retry(RETRY_KEY), b"second"),
            Err(Error::Uncertain),
            "a fenced mount must refuse a retry at operation {step}"
        );
        assert_eq!(
            volume.find_receipt(retry(RETRY_KEY)),
            Err(Error::Uncertain),
            "a fenced mount must not present a receipt at operation {step}"
        );
        let node = *volume.node(5).expect("file node");
        assert_eq!(
            volume.read_file(&mut disk, &node, &mut [0; 8]),
            Err(Error::Uncertain),
            "a fenced mount must not answer a read at operation {step}"
        );
        assert_eq!(
            disk.operations, operations,
            "a fenced mount must not write at operation {step}"
        );

        // Recovering from durable storage finds the generation the header named
        // before the commit: same bytes, same version, no receipt.
        let mut durable = disk.recover();
        let mounted = mount6(&mut durable).expect("the old generation still mounts");
        let node = *mounted.node(5).expect("file node");
        assert_eq!(node.version, 2, "operation {step}");
        assert_eq!(
            bytes_of(&mut durable, &mounted, 5).as_slice(),
            b"first",
            "operation {step}"
        );
        assert_eq!(
            mounted.find_receipt(retry(RETRY_KEY)),
            Ok(None),
            "operation {step}"
        );

        // Mounting the same device again, without a reboot, is a recovery too:
        // it flushes what the device holds before trusting it, so the state it
        // adopts is one whole generation, whether the failed commit had landed
        // or not.
        disk.fail_at = None;
        let mounted = mount6(&mut disk).expect("a remount must recover the device");
        let version = mounted.node(5).expect("file node").version;
        let bytes = bytes_of(&mut disk, &mounted, 5);
        match mounted
            .find_receipt(retry(RETRY_KEY))
            .expect("a recovered mount answers")
            .copied()
        {
            None => {
                assert_eq!(version, 2, "operation {step}");
                assert_eq!(bytes.as_slice(), b"first", "operation {step}");
            }
            Some(receipt) => {
                assert_eq!(version, receipt.committed, "operation {step}");
                assert_eq!(bytes.as_slice(), b"second", "operation {step}");
            }
        }
    }
}

#[test]
fn a_device_that_persists_the_header_early_still_recovers_a_whole_generation() {
    let base = published_first(true);
    let steps = tracked_steps(&base);

    for step in 0..steps {
        let mut disk = base.recover();
        let mut volume = mount6(&mut disk).expect("mount the published generation");
        disk.operations = 0;
        disk.fail_at = Some(step);
        assert_eq!(
            volume.write_tracked(&mut disk, 4, 2, retry(RETRY_KEY), b"second"),
            Err(Error::Uncertain),
            "operation {step} must be Uncertain"
        );

        // Whatever became durable, it is a generation that was flushed before
        // the header named it: the old state, or the new one, never a header
        // over structures that never landed.
        let mut durable = disk.recover();
        let mounted = mount6(&mut durable).expect("a durable state must always mount");
        let found = mounted
            .find_receipt(retry(RETRY_KEY))
            .expect("a mounted volume answers its receipt table")
            .copied();
        let version = mounted.node(5).expect("file node").version;
        let bytes = bytes_of(&mut durable, &mounted, 5);
        match found {
            None => {
                assert_eq!(
                    version, 2,
                    "operation {step} left a version without its receipt"
                );
                assert_eq!(bytes.as_slice(), b"first", "operation {step}");
            }
            Some(receipt) => {
                assert_eq!(
                    version, receipt.committed,
                    "operation {step} published a receipt over another version"
                );
                assert_eq!(bytes.as_slice(), b"second", "operation {step}");
            }
        }
    }
}

/// A disk that records whether the publication header was written while the
/// payload or the inactive generation were still only in the cache. A device
/// may persist that one sector the moment it arrives, so the order the commit
/// issues its writes in is what decides whether a power loss there destroys the
/// volume.
struct Barrier {
    disk: Journal,
    dirty: bool,
    header_before_flush: bool,
}

impl Disk for Barrier {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        self.disk.read(sector, bytes)
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        if sector == HEADER_SECTOR && self.dirty {
            self.header_before_flush = true;
        }
        let result = self.disk.write(sector, bytes);
        if sector != HEADER_SECTOR {
            self.dirty = true;
        }
        result
    }
    fn flush(&mut self) -> Result<(), Error> {
        let result = self.disk.flush();
        if result.is_ok() {
            self.dirty = false;
        }
        result
    }
}

#[test]
fn the_header_is_written_only_after_the_payload_and_the_inactive_generation_are_flushed() {
    let mut disk = Barrier {
        disk: published_first(false),
        dirty: false,
        header_before_flush: false,
    };
    let mut volume = mount6(&mut disk).expect("mount");
    volume
        .write_tracked(&mut disk, 4, 2, retry(RETRY_KEY), b"second")
        .expect("tracked write");
    assert!(
        !disk.header_before_flush,
        "the header may only name a generation that a successful flush already made durable"
    );
    assert!(volume.find_receipt(retry(RETRY_KEY)).is_ok());
}

#[test]
fn a_refused_mount_leaves_no_partially_decoded_state() {
    let mut disk = published_first(false);
    let active = mount6(&mut disk).expect("mount").header.active;
    // The map is read after the node table, so a mount that verifies the header
    // only at the end has already filled the table when the map fails.
    disk.corrupt(map_sector(active), 3);

    let mut slot = Volume6::EMPTY;
    assert_eq!(slot.mount_into(&mut disk), Err(Error::Corrupt));
    assert!(
        slot.node(5).is_none(),
        "a refused mount must not leave a decoded node behind"
    );
    assert_eq!(
        slot.header.sequence, 1,
        "a refused mount publishes no header"
    );
    assert_eq!(
        slot.free_sectors(),
        DATA_SECTORS,
        "a refused mount must not leave a decoded map behind"
    );
    // The slot is not a volume until a mount succeeds: it answers Uncertain.
    assert_eq!(
        slot.find_receipt(retry(RETRY_KEY)),
        Err(Error::Uncertain),
        "a refused mount must not be usable"
    );

    // Mounting the same slot again is the recovery.
    let mut disk = published_first(false);
    slot.mount_into(&mut disk).expect("mount the intact volume");
    assert_eq!(slot.node(5).map(|node| node.version), Some(2));
}

#[test]
fn refusals_before_the_first_write_leave_the_mount_usable() {
    let mut disk = published_first(false);
    let mut volume = mount6(&mut disk).expect("mount");
    let free = volume.free_sectors();
    let operations = disk.operations;

    // Every refusal a caller can cause without touching the device: a stale
    // version, a missing record, an oversized payload and an out-of-range slot.
    assert_eq!(
        volume.write_file(&mut disk, 4, 1, b"stale writer"),
        Err(Error::Version)
    );
    assert_eq!(
        volume.write_file(&mut disk, 9, 0, b"missing record"),
        Err(Error::NotFound)
    );
    assert_eq!(
        volume.write_file(&mut disk, 4, 2, &[0; MAX_FILE_V6 + 1]),
        Err(Error::Size)
    );
    assert_eq!(
        volume.write_file(&mut disk, OBJECTS_V6, 0, b"out of range"),
        Err(Error::Size)
    );
    assert_eq!(disk.operations, operations, "a refusal writes nothing");
    assert_eq!(volume.free_sectors(), free, "a refusal reserves nothing");

    // The mount is not fenced by them: it still commits and publishes.
    assert_eq!(volume.write_file(&mut disk, 4, 2, b"second"), Ok(3));
    let mut disk = disk.recover();
    let mounted = mount6(&mut disk).expect("mount");
    assert_eq!(bytes_of(&mut disk, &mounted, 5).as_slice(), b"second");
}

#[test]
fn a_failed_payload_write_of_an_untracked_write_fences_and_keeps_the_old_bytes() {
    let mut disk = published_first(false);
    let mut volume = mount6(&mut disk).expect("mount");
    disk.operations = 0;
    disk.fail_at = Some(0);
    assert_eq!(
        volume.write_file(&mut disk, 4, 2, b"second"),
        Err(Error::Uncertain)
    );
    disk.fail_at = None;
    let operations = disk.operations;
    assert_eq!(
        volume.write_file(&mut disk, 4, 2, b"second"),
        Err(Error::Uncertain)
    );
    let node = *volume.node(5).expect("file node");
    assert_eq!(
        volume.read_file(&mut disk, &node, &mut [0; 8]),
        Err(Error::Uncertain),
        "a fenced mount answers no read either"
    );
    assert_eq!(disk.operations, operations, "a fenced mount must not write");

    let mut durable = disk.recover();
    let mounted = mount6(&mut durable).expect("mount the published generation");
    assert_eq!(bytes_of(&mut durable, &mounted, 5).as_slice(), b"first");

    // A successful mount restores ordinary operation, and what it accepts is
    // durable.
    let mut healed = mount6(&mut durable).expect("mount again");
    assert_eq!(healed.write_file(&mut durable, 4, 2, b"second"), Ok(3));
    let mut durable = durable.recover();
    let mounted = mount6(&mut durable).expect("mount the recovered write");
    assert_eq!(mounted.node(5).map(|node| node.version), Some(3));
    assert_eq!(bytes_of(&mut durable, &mounted, 5).as_slice(), b"second");
}

#[test]
fn a_failed_final_flush_of_a_removal_leaves_the_file_and_fences() {
    // Measure a removal's operations, then fail its last one: the final flush
    // that would have made the removal durable.
    let mut probe = published_first(false);
    let mut probed = mount6(&mut probe).expect("mount");
    probe.operations = 0;
    probed.remove_file(&mut probe, 4).expect("remove");
    let steps = probe.operations;

    let mut disk = published_first(false);
    let mut volume = mount6(&mut disk).expect("mount");
    disk.operations = 0;
    disk.fail_at = Some(steps - 1);
    assert_eq!(volume.remove_file(&mut disk, 4), Err(Error::Uncertain));
    disk.fail_at = None;
    assert_eq!(volume.remove_file(&mut disk, 4), Err(Error::Uncertain));

    let mut durable = disk.recover();
    let mounted = mount6(&mut durable).expect("mount the published generation");
    assert_eq!(
        bytes_of(&mut durable, &mounted, 5).as_slice(),
        b"first",
        "a removal that never settled must not have happened"
    );
    assert_eq!(mounted.free_sectors(), DATA_SECTORS - 1);
}

#[test]
fn a_commit_that_never_settled_must_not_replay_its_receipt() {
    let base = published_first(false);
    let steps = tracked_steps(&base);

    // The header write and the final flush are the operations after which the
    // volume still holds a fully built commit in memory but the receipt is not
    // durable. A replay that answers from that memory tells the caller an
    // operation is committed when the volume would not find it again.
    for step in [steps - 2, steps - 1] {
        let mut disk = base.recover();
        let mut volume = mount6(&mut disk).expect("mount");
        disk.operations = 0;
        disk.fail_at = Some(step);
        assert_eq!(
            volume.write_tracked(&mut disk, 4, 2, retry(RETRY_KEY), b"second"),
            Err(Error::Uncertain),
            "operation {step}"
        );
        disk.fail_at = None;
        assert_eq!(
            volume.write_tracked(&mut disk, 4, 2, retry(RETRY_KEY), b"second"),
            Err(Error::Uncertain),
            "a replay after operation {step} must not report the unsettled commit"
        );
        let mut durable = disk.recover();
        let mounted = mount6(&mut durable).expect("mount");
        assert_eq!(
            mounted.find_receipt(retry(RETRY_KEY)),
            Ok(None),
            "operation {step} published no receipt"
        );
    }
}

#[test]
fn a_failed_attempt_must_not_leak_the_payload_it_reserved() {
    let mut disk = published_first(false);
    let mut volume = mount6(&mut disk).expect("mount");
    let free = volume.free_sectors();
    disk.operations = 0;
    disk.fail_at = Some(0);
    assert_eq!(
        volume.write_file(&mut disk, 4, 2, b"second"),
        Err(Error::Uncertain)
    );
    disk.fail_at = None;
    // Whatever the fence answers, the map the failed attempt touched must not
    // have kept the runs it reserved: a caller that retries must not pay twice.
    let _ = volume.write_file(&mut disk, 4, 2, b"second");
    let mut durable = disk.recover();
    let mounted = mount6(&mut durable).expect("mount");
    assert_eq!(
        mounted.free_sectors(),
        free,
        "the failed attempt leaked the payload it reserved"
    );
    assert_eq!(bytes_of(&mut durable, &mounted, 5).as_slice(), b"first");
}

#[test]
fn a_remount_adopts_only_a_generation_it_made_durable() {
    let base = published_first(false);
    let steps = tracked_steps(&base);
    let mut disk = base.recover();
    {
        let mut volume = mount6(&mut disk).expect("mount");
        disk.operations = 0;
        // The final flush fails, so the successful header write above it is in
        // the device's volatile cache and the durable header still names the old
        // generation.
        disk.fail_at = Some(steps - 1);
        assert_eq!(
            volume.write_tracked(&mut disk, 4, 2, retry(RETRY_KEY), b"second"),
            Err(Error::Uncertain)
        );
    }
    disk.fail_at = None;

    // Remounting the same device, without a reboot, is a recovery: the mount
    // makes what the cache holds durable before it adopts it.
    let mounted = mount6(&mut disk).expect("remount the live device");
    let adopted = mounted
        .find_receipt(retry(RETRY_KEY))
        .expect("a recovered mount answers")
        .copied();
    assert_eq!(adopted.map(|receipt| receipt.committed), Some(3));

    // Drop the volatile cache: whatever was adopted has to be in the durable
    // generation, or the mount trusted a state the device never held.
    let mut rebooted = disk.recover();
    let mounted = mount6(&mut rebooted).expect("mount after the cache is dropped");
    assert_eq!(
        mounted
            .find_receipt(retry(RETRY_KEY))
            .expect("a recovered mount answers")
            .copied(),
        adopted,
        "the adopted receipt must survive dropping the cache"
    );
    assert_eq!(
        bytes_of(&mut rebooted, &mounted, 5).as_slice(),
        b"second",
        "the adopted payload must survive dropping the cache"
    );
}

#[test]
fn a_failed_recovery_flush_leaves_the_value_fenced_and_unread() {
    let mut disk = published_first(false);
    let mut volume = Volume6::EMPTY;
    // The mount's own flush is its first operation, so it is the one that fails
    // here: a device that cannot make what it holds durable is not a recovery
    // source, and the value must not decode or mutate anything.
    disk.fail_at = Some(0);
    assert_eq!(volume.mount_into(&mut disk), Err(Error::Io));
    assert!(volume.node(5).is_none(), "a failed flush decoded nothing");
    assert_eq!(volume.header.sequence, 1);
    assert_eq!(volume.free_sectors(), DATA_SECTORS);
    assert_eq!(
        volume.find_receipt(retry(RETRY_KEY)),
        Err(Error::Uncertain),
        "a failed recovery flush must leave the value fenced"
    );

    disk.fail_at = None;
    let operations = disk.operations;
    assert_eq!(
        volume.write_file(&mut disk, 4, 2, b"second"),
        Err(Error::Uncertain)
    );
    assert_eq!(disk.operations, operations, "a fenced value must not write");

    // Mounting again, with a flush that succeeds, is the recovery.
    volume.mount_into(&mut disk).expect("mount again");
    assert_eq!(volume.node(5).map(|node| node.version), Some(2));
    assert_eq!(volume.write_file(&mut disk, 4, 2, b"second"), Ok(3));
}
