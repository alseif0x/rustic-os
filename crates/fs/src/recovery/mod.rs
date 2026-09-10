// SPDX-License-Identifier: Apache-2.0
//! Bounded durable replacement evidence, independent from client authorization.
mod codec;
mod operations;
use crate::{Error, MAX_FILE};
pub const RETAINED: usize = 2;
pub const RECOVERY_SECTORS: usize = 7;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retry {
    pub lineage: [u8; 16],
    pub epoch: u64,
    pub key: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub retry: Retry,
    pub id: u32,
    pub previous: u64,
    pub committed: u64,
    pub length: u16,
}
#[derive(Clone, Copy)]
pub(super) struct Record {
    pub(super) subject: u64,
    pub(super) receipt: Receipt,
    pub(super) bytes: [u8; MAX_FILE],
}
#[derive(Clone)]
pub(super) struct Recovery {
    pub(super) lineage: [u8; 16],
    pub(super) epoch: u64,
    pub(super) records: [Option<Record>; RETAINED],
}
impl Recovery {
    pub(super) fn new(lineage: [u8; 16]) -> Result<Self, Error> {
        if lineage == [0; 16] {
            return Err(Error::Invalid);
        }
        Ok(Self {
            lineage,
            epoch: 1,
            records: [None; RETAINED],
        })
    }
    pub(super) fn find(&self, subject: u64, retry: Retry) -> Result<Option<&Record>, Error> {
        if subject == 0 || retry.key == 0 || retry.epoch == 0 {
            return Err(Error::Invalid);
        }
        if retry.lineage != self.lineage {
            return Err(Error::Lineage);
        }
        if let Some(record) = self
            .records
            .iter()
            .flatten()
            .find(|r| r.subject == subject && r.receipt.retry == retry)
        {
            return Ok(Some(record));
        }
        if retry.epoch != self.epoch {
            return Err(Error::ExpiredEpoch);
        }
        Ok(None)
    }
}
