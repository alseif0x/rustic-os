// SPDX-License-Identifier: Apache-2.0
//! The v7 durable record: a scoped admission outcome with the payload extent
//! snapshot that makes an exact-byte retry answerable.
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0 | 8 | subject |
//! | 8 | 4 | workspace |
//! | 12 | 4 | object |
//! | 16 | 8 | instance |
//! | 24 | 8 | retry epoch |
//! | 32 | 8 | retry key |
//! | 40 | 8 | previous |
//! | 48 | 8 | committed |
//! | 56 | 8 | admission number |
//! | 64 | 8 | terminal |
//! | 72 | 4 | length |
//! | 76 | 4 | payload CRC-32 |
//! | 80 | 1 | state |
//! | 81 | 1 | prevention cause |
//! | 82 | 1 | extents used |
//! | 83 | 5 | reserved zero |
//! | 88 | 64 | eight `(start, sectors)` u32 pairs |
//! | 152 | 36 | reserved zero |
//! | 188 | 4 | CRC-32 over bytes 0..188 |
//!
//! The lineage is not here: it is shared, and lives in the header. A record
//! proves its scope, its state arithmetic and its payload geometry by itself;
//! [`Record7::validate`] adds the bounds that need the volume sequence and the
//! identity watermark. An empty slot is 192 zero bytes, which this decoder
//! refuses: [`Record7::slot`] and [`receipt_slots`] are what turn a zero slot
//! into "empty".

use super::header::NEXT_MIN;
use super::{MAX_EXTENTS, RECEIPT_BLOCK_BYTES, RECORD_BYTES, RETAINED, check_payload};
use crate::Error;
use crate::admission::PreventionReason;
use crate::checksum::crc;
use crate::extent::Extent;

/// Reserved identities: 1..=4 name the roots, so a scoped record's object is
/// always above them.
pub const FIRST_OBJECT: u32 = 5;

/// The four durable outcomes a record can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RecordState {
    /// A direct commit with no admission: `admission number` is zero and the
    /// terminal sequence is the committed one.
    DirectCommitted = 0,
    /// An admission is recorded but nothing is committed or terminal yet.
    Admitted = 1,
    /// The admission was cancelled with a retained cause; nothing committed.
    Cancelled = 2,
    /// An admitted operation reached its terminal sequence.
    AdmittedCommitted = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record7 {
    pub subject: u64,
    pub workspace: u32,
    pub object: u32,
    pub instance: u64,
    pub retry_epoch: u64,
    pub retry_key: u64,
    pub previous: u64,
    pub committed: u64,
    pub admission_number: u64,
    pub terminal: u64,
    pub length: u32,
    pub payload_crc32: u32,
    pub state: RecordState,
    /// Present exactly for [`RecordState::Cancelled`], where all four existing
    /// distinctions are preserved, including "the cause was never recorded".
    pub prevention: Option<PreventionReason>,
    pub extents_used: u8,
    pub extents: [Extent; MAX_EXTENTS],
}

impl Record7 {
    pub fn runs(&self) -> &[Extent] {
        &self.extents[..usize::from(self.extents_used)]
    }

    /// Serialize a record this codec would read back, refusing an invalid one
    /// rather than persisting bytes its own decoder refuses.
    pub fn encode(&self) -> Result<[u8; RECORD_BYTES], Error> {
        self.check_local()?;
        let mut b = [0; RECORD_BYTES];
        b[0..8].copy_from_slice(&self.subject.to_le_bytes());
        b[8..12].copy_from_slice(&self.workspace.to_le_bytes());
        b[12..16].copy_from_slice(&self.object.to_le_bytes());
        b[16..24].copy_from_slice(&self.instance.to_le_bytes());
        b[24..32].copy_from_slice(&self.retry_epoch.to_le_bytes());
        b[32..40].copy_from_slice(&self.retry_key.to_le_bytes());
        b[40..48].copy_from_slice(&self.previous.to_le_bytes());
        b[48..56].copy_from_slice(&self.committed.to_le_bytes());
        b[56..64].copy_from_slice(&self.admission_number.to_le_bytes());
        b[64..72].copy_from_slice(&self.terminal.to_le_bytes());
        b[72..76].copy_from_slice(&self.length.to_le_bytes());
        b[76..80].copy_from_slice(&self.payload_crc32.to_le_bytes());
        b[80] = self.state as u8;
        b[81] = self.prevention.map_or(0, |reason| reason as u8);
        b[82] = self.extents_used;
        for (index, run) in self.extents.iter().enumerate() {
            let at = 88 + index * 8;
            b[at..at + 4].copy_from_slice(&(run.start as u32).to_le_bytes());
            b[at + 4..at + 8].copy_from_slice(&(run.sectors as u32).to_le_bytes());
        }
        let checksum = crc(&b[..188]);
        b[188..].copy_from_slice(&checksum.to_le_bytes());
        Ok(b)
    }

    /// Strict record decoding. A zero slot is refused here; use [`Self::slot`].
    pub fn decode(b: &[u8; RECORD_BYTES]) -> Result<Self, Error> {
        let checksum = u32::from_le_bytes(b[188..192].try_into().unwrap());
        if crc(&b[..188]) != checksum
            || b[83..88] != [0; 5]
            || b[152..188].iter().any(|byte| *byte != 0)
        {
            return Err(Error::Corrupt);
        }
        let state = match b[80] {
            0 => RecordState::DirectCommitted,
            1 => RecordState::Admitted,
            2 => RecordState::Cancelled,
            3 => RecordState::AdmittedCommitted,
            _ => return Err(Error::Corrupt),
        };
        let prevention = if state == RecordState::Cancelled {
            Some(match b[81] {
                0 => PreventionReason::Unknown,
                1 => PreventionReason::Requested,
                2 => PreventionReason::VersionConflict,
                3 => PreventionReason::AuthorityLost,
                _ => return Err(Error::Corrupt),
            })
        } else {
            if b[81] != 0 {
                return Err(Error::Corrupt);
            }
            None
        };
        let mut extents = [Extent::new(0, 0); MAX_EXTENTS];
        for (index, run) in extents.iter_mut().enumerate() {
            let at = 88 + index * 8;
            *run = Extent::new(
                u64::from(u32::from_le_bytes(b[at..at + 4].try_into().unwrap())),
                u64::from(u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap())),
            );
        }
        let record = Self {
            subject: u64::from_le_bytes(b[0..8].try_into().unwrap()),
            workspace: u32::from_le_bytes(b[8..12].try_into().unwrap()),
            object: u32::from_le_bytes(b[12..16].try_into().unwrap()),
            instance: u64::from_le_bytes(b[16..24].try_into().unwrap()),
            retry_epoch: u64::from_le_bytes(b[24..32].try_into().unwrap()),
            retry_key: u64::from_le_bytes(b[32..40].try_into().unwrap()),
            previous: u64::from_le_bytes(b[40..48].try_into().unwrap()),
            committed: u64::from_le_bytes(b[48..56].try_into().unwrap()),
            admission_number: u64::from_le_bytes(b[56..64].try_into().unwrap()),
            terminal: u64::from_le_bytes(b[64..72].try_into().unwrap()),
            length: u32::from_le_bytes(b[72..76].try_into().unwrap()),
            payload_crc32: u32::from_le_bytes(b[76..80].try_into().unwrap()),
            state,
            prevention,
            extents_used: b[82],
            extents,
        };
        record.check_local()?;
        Ok(record)
    }

    /// One retained slot: `None` for the all-zero slot a caller did not fill,
    /// otherwise the strict record.
    pub fn slot(b: &[u8; RECORD_BYTES]) -> Result<Option<Self>, Error> {
        if b == &[0; RECORD_BYTES] {
            Ok(None)
        } else {
            Ok(Some(Self::decode(b)?))
        }
    }

    /// What one record can prove about itself: a scoped identity, a retry with a
    /// key and an epoch no newer than the operation it belongs to, a metadata
    /// version it actually replaces, a prevention cause exactly where the state
    /// allows one, the state's own arithmetic, and payload runs that describe the
    /// length exactly. The replaced version is never zero, the rule the v5
    /// recovery codec and the ABI version type already keep.
    fn check_local(&self) -> Result<(), Error> {
        if self.subject == 0
            || self.workspace == 0
            || self.instance == 0
            || self.object < FIRST_OBJECT
            || self.object == self.workspace
            || self.retry_key == 0
            || self.retry_epoch == 0
            || self.previous == 0
        {
            return Err(Error::Corrupt);
        }
        let allowed_prevention = self.state == RecordState::Cancelled;
        if allowed_prevention != self.prevention.is_some() {
            return Err(Error::Corrupt);
        }
        match self.state {
            // Direct commit: no admission, the committed sequence is both the
            // terminal one and what the instance belongs to.
            RecordState::DirectCommitted
                if self.admission_number != 0
                    || self.terminal != self.committed
                    || self.committed <= self.previous
                    || self.instance > self.committed =>
            {
                return Err(Error::Corrupt);
            }
            // Admitted: nothing is committed or terminal yet.
            RecordState::Admitted
                if self.committed != 0
                    || self.terminal != 0
                    || self.admission_number <= self.previous
                    || self.instance > self.admission_number =>
            {
                return Err(Error::Corrupt);
            }
            // Cancelled: the terminal sequence follows uncommitted admission.
            RecordState::Cancelled
                if self.committed != 0
                    || self.terminal <= self.admission_number
                    || self.admission_number <= self.previous
                    || self.instance > self.admission_number =>
            {
                return Err(Error::Corrupt);
            }
            // Admitted and committed: the terminal sequence is the committed one.
            RecordState::AdmittedCommitted
                if self.committed != self.terminal
                    || self.terminal <= self.admission_number
                    || self.admission_number <= self.previous
                    || self.instance > self.admission_number =>
            {
                return Err(Error::Corrupt);
            }
            _ => (),
        }
        // The retry epoch belongs to the operation that created it, not to the
        // volume: a direct commit answers on its committed sequence, and an
        // admitted operation answers on the admission it was created for. The
        // sequence bound in `validate` is the outer limit, not this one.
        let epoch_bound = match self.state {
            RecordState::DirectCommitted => self.committed,
            RecordState::Admitted | RecordState::Cancelled | RecordState::AdmittedCommitted => {
                self.admission_number
            }
        };
        if self.retry_epoch > epoch_bound {
            return Err(Error::Corrupt);
        }
        check_payload(&self.extents, self.extents_used, self.length)
    }

    /// The contextual bounds, which need the header the record belongs to:
    /// `sequence` is the global transaction/version identity and `next` is the
    /// identity watermark.
    ///
    /// Workspace and object are identities allocated below the watermark, so they
    /// are compared against `next` and never against the node table's 256 slots;
    /// a volume that has cycled more identities than it can hold live is exactly
    /// what the watermark exists for. Instance, epoch and the sequences are
    /// version-domain numbers: they are bounded by `sequence`, while the epoch is
    /// already bounded locally to the operation that created it.
    pub fn validate(&self, sequence: u64, next: u32) -> Result<(), Error> {
        if sequence == 0 || next < NEXT_MIN {
            return Err(Error::Corrupt);
        }
        self.check_local()?;
        if self.workspace >= next || self.object >= next {
            return Err(Error::Corrupt);
        }
        if self.retry_epoch > sequence
            || self.previous > sequence
            || self.committed > sequence
            || self.terminal > sequence
            || self.admission_number > sequence
        {
            return Err(Error::Corrupt);
        }
        Ok(())
    }
}

/// Decode one generation's retained receipt block. The eight slots come first and
/// the padding after them is reserved, so a non-zero byte there is corruption.
pub fn receipt_slots(
    block: &[u8; RECEIPT_BLOCK_BYTES],
) -> Result<[Option<Record7>; RETAINED], Error> {
    if block[RETAINED * RECORD_BYTES..]
        .iter()
        .any(|byte| *byte != 0)
    {
        return Err(Error::Corrupt);
    }
    let mut slots = [None; RETAINED];
    for (index, slot) in slots.iter_mut().enumerate() {
        let at = index * RECORD_BYTES;
        *slot = Record7::slot(&block[at..at + RECORD_BYTES].try_into().unwrap())?;
    }
    Ok(slots)
}
