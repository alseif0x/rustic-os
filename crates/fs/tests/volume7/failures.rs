// SPDX-License-Identifier: Apache-2.0
//! Failure and remount behavior for v7 provisioning and read-only mounting.

use super::{LINEAGE, support::Sparse};
use rustic_fs::format7::{self, Header7};
use rustic_fs::{Disk, Error, Volume7};

struct ReadFailure<'a>(&'a mut Sparse);

impl Disk for ReadFailure<'_> {
    fn read(&mut self, _: u64, _: &mut [u8; 512]) -> Result<(), Error> {
        Err(Error::Io)
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        self.0.write(sector, bytes)
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.0.flush()
    }
}

#[test]
fn every_provision_write_and_flush_failure_leaves_no_durable_mountable_head() {
    // Two header invalidations, 64 node sectors, 32 map sectors, four receipt
    // sectors, one header write, and three flush barriers.
    const PROVISION_STEPS: usize = 106;

    for failure in 0..PROVISION_STEPS {
        let mut disk = Sparse {
            fail_at: Some(failure),
            ..Sparse::default()
        };
        let mut volume = Volume7::EMPTY;
        assert_eq!(
            volume.provision_into(&mut disk, LINEAGE),
            Err(Error::Io),
            "provision step {failure} should fail"
        );
        assert_eq!(volume.header().err(), Some(Error::Uncertain));
        assert_eq!(volume.node(1).err(), Some(Error::Uncertain));
        assert_eq!(volume.free_sectors().err(), Some(Error::Uncertain));

        disk.fail_at = None;
        let mut durable_disk = disk.recover();
        durable_disk.fail_at = None;
        let mut recovered = Volume7::EMPTY;
        assert_eq!(
            recovered.mount_into(&mut durable_disk),
            Err(Error::Corrupt),
            "durable media after failed provision step {failure} must not mount"
        );
    }
}

#[test]
fn failed_remount_clears_access_and_a_later_successful_mount_recovers() {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    assert_eq!(volume.node(1).unwrap().unwrap().name(), b"system");

    let mut durable_disk = disk.recover();
    durable_disk.fail_at = Some(0);
    assert_eq!(volume.mount_into(&mut durable_disk), Err(Error::Io));
    assert_eq!(volume.header().err(), Some(Error::Uncertain));
    assert_eq!(volume.node(1).err(), Some(Error::Uncertain));

    durable_disk.fail_at = None;
    volume.mount_into(&mut durable_disk).unwrap();
    assert_eq!(volume.node(1).unwrap().unwrap().name(), b"system");

    assert_eq!(
        volume.mount_into(&mut ReadFailure(&mut durable_disk)),
        Err(Error::Io)
    );
    assert_eq!(volume.header().err(), Some(Error::Uncertain));
    assert_eq!(volume.free_sectors().err(), Some(Error::Uncertain));

    volume.mount_into(&mut durable_disk).unwrap();
    assert_eq!(volume.free_sectors(), Ok(rustic_fs::DATA_SECTORS));
}

struct TornWrite<'a> {
    disk: &'a mut Sparse,
    fail_at: usize,
    operations: usize,
    prefix: usize,
}

impl TornWrite<'_> {
    fn step(&mut self) -> bool {
        let fail = self.operations == self.fail_at;
        self.operations += 1;
        fail
    }
}

impl Disk for TornWrite<'_> {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        self.operations += 1;
        self.disk.read(sector, bytes)
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        if self.step() {
            let mut torn = self.disk.live.get(&sector).copied().unwrap_or([0; 512]);
            torn[..self.prefix].copy_from_slice(&bytes[..self.prefix]);
            self.disk.live.insert(sector, torn);
            self.disk.durable.insert(sector, torn);
            return Err(Error::Io);
        }
        self.disk.write(sector, bytes)
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.operations += 1;
        self.disk.flush()
    }
}

#[test]
fn a_torn_first_header_write_is_not_mountable_after_durable_recovery() {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    let result = volume.provision_into(
        &mut TornWrite {
            disk: &mut disk,
            fail_at: 104,
            operations: 0,
            prefix: 256,
        },
        LINEAGE,
    );
    assert_eq!(result, Err(Error::Io));
    assert_eq!(volume.header().err(), Some(Error::Uncertain));

    let torn_header = *disk.live.get(&format7::header_sector(0)).unwrap();
    assert!(Header7::decode(&torn_header).is_err());
    let mut rebooted = disk.recover();
    assert_eq!(
        rebooted.live.get(&format7::header_sector(0)),
        Some(&torn_header)
    );
    let mut mounted = Volume7::EMPTY;
    assert_eq!(mounted.mount_into(&mut rebooted), Err(Error::Corrupt));
}
