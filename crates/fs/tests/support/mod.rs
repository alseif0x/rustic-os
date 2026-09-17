// SPDX-License-Identifier: Apache-2.0
//! Shared host disks. Every test binary compiles this module and uses a subset of
//! it, so unused fixtures are expected rather than a defect.
#![allow(dead_code)]
use rustic_fs::{Disk, Error, SECTORS};
#[derive(Clone)]
pub struct MemoryDisk {
    pub live: Vec<[u8; 512]>,
    pub durable: Vec<[u8; 512]>,
    pub operations: usize,
    pub fail: Option<usize>,
    pub tear: usize,
}
impl MemoryDisk {
    pub fn new() -> Self {
        Self {
            live: vec![[0; 512]; SECTORS as usize],
            durable: vec![[0; 512]; SECTORS as usize],
            operations: 0,
            fail: None,
            tear: 0,
        }
    }
    pub fn recover(&self, durable: bool) -> Self {
        let mut d = Self::new();
        d.live = if durable {
            self.durable.clone()
        } else {
            self.live.clone()
        };
        d.durable = d.live.clone();
        d
    }
    fn step(&mut self) -> bool {
        let fail = self.fail == Some(self.operations);
        self.operations += 1;
        fail
    }
}
impl Disk for MemoryDisk {
    fn read(&mut self, sector: u64, b: &mut [u8; 512]) -> Result<(), Error> {
        *b = *self.live.get(sector as usize).ok_or(Error::Io)?;
        Ok(())
    }
    fn write(&mut self, sector: u64, b: &[u8; 512]) -> Result<(), Error> {
        if self.step() {
            self.live[sector as usize][..self.tear].copy_from_slice(&b[..self.tear]);
            return Err(Error::Io);
        }
        self.live[sector as usize] = *b;
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

/// Sparse disk for the 64 MiB v6 payload region: only written sectors are kept,
/// so an unused payload costs nothing. `durable` models flush and `fail_at`
/// fails the write or flush with the given operation number.
#[derive(Default)]
pub struct Sparse {
    pub live: std::collections::BTreeMap<u64, [u8; 512]>,
    pub durable: std::collections::BTreeMap<u64, [u8; 512]>,
    pub operations: usize,
    pub fail_at: Option<usize>,
}

impl Sparse {
    pub fn recover(&self) -> Self {
        Self {
            live: self.durable.clone(),
            durable: self.durable.clone(),
            operations: 0,
            fail_at: self.fail_at,
        }
    }
    pub fn corrupt(&mut self, sector: u64, offset: usize) {
        self.live.get_mut(&sector).expect("written sector")[offset] ^= 1;
    }
    fn step(&mut self) -> bool {
        let fail = self.fail_at == Some(self.operations);
        self.operations += 1;
        fail
    }
}

impl Disk for Sparse {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        *bytes = self.live.get(&sector).copied().unwrap_or([0; 512]);
        Ok(())
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        if self.step() {
            return Err(Error::Io);
        }
        self.live.insert(sector, *bytes);
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
