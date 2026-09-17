// SPDX-License-Identifier: Apache-2.0
//! Bounded v6 operation receipts (#51).
//!
//! A receipt is the durable evidence that an effect may have happened. The v5
//! volume retains two of them; the selected v6 budget keeps eight, and the rules
//! that matter are the ones this module refuses to break: a full table reports
//! `Full` instead of evicting the oldest, and an epoch cannot be rotated while a
//! retained record would be dropped, because that would make a client's
//! unresolved outcome look like it never happened.

use crate::checksum::{crc, crc_update};
use crate::recovery::Retry;
use crate::{Error, OBJECTS_V6};

/// Receipts the v6 volume retains.
pub const RETAINED_V6: usize = 8;
/// On-disk size of one receipt record.
pub const RECEIPT_BYTES: usize = 96;
/// Sectors one generation's receipt block occupies: a 24-byte block header, the
/// records, and padding to a sector boundary.
pub const BLOCK_BYTES: usize = 1024;
pub const RECEIPT_SECTORS: u64 = BLOCK_BYTES as u64 / 512;
/// Bytes before the records inside the block: lineage and epoch.
pub const BLOCK_HEADER_BYTES: usize = 24;

/// What a committed operation records, without the payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receipt6 {
    pub retry: Retry,
    pub id: u32,
    pub previous: u64,
    pub committed: u64,
    /// 32 bits, matching the v6 record's widened length.
    pub length: u32,
}

impl Receipt6 {
    pub fn encode(&self) -> [u8; RECEIPT_BYTES] {
        let mut b = [0; RECEIPT_BYTES];
        b[..16].copy_from_slice(&self.retry.lineage);
        b[16..24].copy_from_slice(&self.retry.epoch.to_le_bytes());
        b[24..32].copy_from_slice(&self.retry.key.to_le_bytes());
        b[32..36].copy_from_slice(&self.id.to_le_bytes());
        b[36..44].copy_from_slice(&self.previous.to_le_bytes());
        b[44..52].copy_from_slice(&self.committed.to_le_bytes());
        b[52..56].copy_from_slice(&self.length.to_le_bytes());
        let checksum = crc(&b[..RECEIPT_BYTES - 4]);
        b[RECEIPT_BYTES - 4..].copy_from_slice(&checksum.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8; RECEIPT_BYTES]) -> Result<Self, Error> {
        let checksum = u32::from_le_bytes(b[RECEIPT_BYTES - 4..].try_into().unwrap());
        let mut body = *b;
        body[RECEIPT_BYTES - 4..].fill(0);
        if crc(&body[..RECEIPT_BYTES - 4]) != checksum || b[56..92].iter().any(|byte| *byte != 0) {
            return Err(Error::Corrupt);
        }
        let lineage: [u8; 16] = b[..16].try_into().unwrap();
        let epoch = u64::from_le_bytes(b[16..24].try_into().unwrap());
        let key = u64::from_le_bytes(b[24..32].try_into().unwrap());
        if lineage == [0; 16] || epoch == 0 || key == 0 {
            return Err(Error::Corrupt);
        }
        let id = u32::from_le_bytes(b[32..36].try_into().unwrap());
        if id == 0 || id > OBJECTS_V6 as u32 {
            return Err(Error::Corrupt);
        }
        let previous = u64::from_le_bytes(b[36..44].try_into().unwrap());
        let committed = u64::from_le_bytes(b[44..52].try_into().unwrap());
        if committed <= previous {
            return Err(Error::Corrupt);
        }
        Ok(Self {
            retry: Retry {
                lineage,
                epoch,
                key,
            },
            id,
            previous,
            committed,
            length: u32::from_le_bytes(b[52..56].try_into().unwrap()),
        })
    }
}

/// The retained receipts of one lineage, with the current epoch.
#[derive(Clone, Copy)]
pub struct Receipts6 {
    lineage: [u8; 16],
    epoch: u64,
    slots: [Option<Receipt6>; RETAINED_V6],
}

impl Receipts6 {
    /// An empty table with no lineage, for a volume that has not been read yet.
    pub const EMPTY: Self = Self {
        lineage: [0; 16],
        epoch: 0,
        slots: [None; RETAINED_V6],
    };
    pub fn new(lineage: [u8; 16]) -> Result<Self, Error> {
        if lineage == [0; 16] {
            return Err(Error::Invalid);
        }
        Ok(Self {
            lineage,
            epoch: 1,
            slots: [None; RETAINED_V6],
        })
    }
    /// The lineage the table is bound to, which is also the volume's identity.
    pub fn lineage(&self) -> [u8; 16] {
        self.lineage
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn records(&self) -> impl Iterator<Item = &Receipt6> + '_ {
        self.slots.iter().flatten()
    }
    /// The retained record for this identity, or `None` for a new operation.
    /// A wrong lineage and a stale epoch are refusals, not misses.
    pub fn find(&self, retry: Retry) -> Result<Option<&Receipt6>, Error> {
        if retry.key == 0 || retry.epoch == 0 {
            return Err(Error::Invalid);
        }
        if retry.lineage != self.lineage {
            return Err(Error::Lineage);
        }
        if let Some(record) = self.slots.iter().flatten().find(|r| r.retry == retry) {
            return Ok(Some(record));
        }
        if retry.epoch != self.epoch {
            return Err(Error::ExpiredEpoch);
        }
        Ok(None)
    }
    /// Retain a receipt. Repeating an identity returns the record already held,
    /// so a retry is idempotent; a full table is `Full`, never an eviction.
    pub fn retain(&mut self, receipt: Receipt6) -> Result<(), Error> {
        if receipt.retry.lineage != self.lineage {
            return Err(Error::Lineage);
        }
        if receipt.retry.epoch != self.epoch {
            return Err(Error::ExpiredEpoch);
        }
        if self
            .slots
            .iter()
            .flatten()
            .any(|r| r.retry == receipt.retry)
        {
            return Ok(());
        }
        let slot = self.slots.iter_mut().find(|slot| slot.is_none());
        match slot {
            Some(slot) => {
                *slot = Some(receipt);
                Ok(())
            }
            None => Err(Error::Full),
        }
    }
    /// Rotate to a later epoch. Refused while any record is retained, because the
    /// rotation is what would make those outcomes unknowable; the caller must
    /// purge them explicitly and at a point of its own choosing.
    pub fn roll_epoch(&mut self, epoch: u64) -> Result<(), Error> {
        if epoch <= self.epoch {
            return Err(Error::Invalid);
        }
        if !self.is_empty() {
            return Err(Error::Full);
        }
        self.epoch = epoch;
        Ok(())
    }
    /// The explicit maintenance path: drop retained records and return how many
    /// were released, so the caller can report what it discarded.
    pub fn purge(&mut self) -> usize {
        let dropped = self.len();
        self.slots = [None; RETAINED_V6];
        dropped
    }
    /// The table's checksum, computed incrementally so it needs no temporary.
    pub fn checksum(&self) -> u32 {
        let mut state = !0u32;
        crc_update(&mut state, &self.lineage);
        crc_update(&mut state, &self.epoch.to_le_bytes());
        for slot in &self.slots {
            match slot {
                Some(record) => crc_update(&mut state, &record.encode()),
                None => crc_update(&mut state, &[0; RECEIPT_BYTES]),
            }
        }
        !state
    }
    /// Serialize the table for one generation's receipt sectors.
    pub fn encode(&self) -> [u8; RETAINED_V6 * RECEIPT_BYTES] {
        let mut bytes = [0; RETAINED_V6 * RECEIPT_BYTES];
        for (index, slot) in self.slots.iter().enumerate() {
            if let Some(record) = slot {
                let at = index * RECEIPT_BYTES;
                bytes[at..at + RECEIPT_BYTES].copy_from_slice(&record.encode());
            }
        }
        bytes
    }
    /// Serialize the whole block, including the lineage and epoch it belongs to,
    /// so a generation's receipts are self-describing.
    pub fn encode_block(&self) -> [u8; BLOCK_BYTES] {
        let mut bytes = [0; BLOCK_BYTES];
        bytes[..16].copy_from_slice(&self.lineage);
        bytes[16..24].copy_from_slice(&self.epoch.to_le_bytes());
        bytes[BLOCK_HEADER_BYTES..BLOCK_HEADER_BYTES + RETAINED_V6 * RECEIPT_BYTES]
            .copy_from_slice(&self.encode());
        bytes
    }
    pub fn decode_block(bytes: &[u8; BLOCK_BYTES]) -> Result<Self, Error> {
        if bytes[BLOCK_HEADER_BYTES + RETAINED_V6 * RECEIPT_BYTES..]
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err(Error::Corrupt);
        }
        let lineage: [u8; 16] = bytes[..16].try_into().unwrap();
        let epoch = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        let mut records = [0u8; RETAINED_V6 * RECEIPT_BYTES];
        records.copy_from_slice(
            &bytes[BLOCK_HEADER_BYTES..BLOCK_HEADER_BYTES + RETAINED_V6 * RECEIPT_BYTES],
        );
        Self::decode(lineage, epoch, &records)
    }
    pub fn decode(
        lineage: [u8; 16],
        epoch: u64,
        bytes: &[u8; RETAINED_V6 * RECEIPT_BYTES],
    ) -> Result<Self, Error> {
        if lineage == [0; 16] || epoch == 0 {
            return Err(Error::Corrupt);
        }
        let mut slots = [None; RETAINED_V6];
        for (index, slot) in slots.iter_mut().enumerate() {
            let at = index * RECEIPT_BYTES;
            let record: [u8; RECEIPT_BYTES] = bytes[at..at + RECEIPT_BYTES].try_into().unwrap();
            if record == [0; RECEIPT_BYTES] {
                continue;
            }
            let decoded = Receipt6::decode(&record)?;
            if decoded.retry.lineage != lineage {
                return Err(Error::Corrupt);
            }
            *slot = Some(decoded);
        }
        Ok(Self {
            lineage,
            epoch,
            slots,
        })
    }
}
