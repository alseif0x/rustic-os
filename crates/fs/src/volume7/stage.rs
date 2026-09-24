// SPDX-License-Identifier: Apache-2.0
//! Owner-side streamed staging of one v7 payload with one sector of memory.
//!
//! A stage lets a caller supply a file of up to [`MAX_FILE_BYTES`] one sector at
//! a time instead of lending the whole candidate. A fresh stage reserves its
//! planned payload runs and one receipt slot in memory only: the reservation is
//! an overlay that other planners avoid, and it is never written into the
//! mounted allocation map, which must keep describing exact durable ownership.
//! Abort, [`Volume7::release_stages`], remount and any fence release every
//! reservation without I/O. A stage
//! that matches a retained retry record reserves nothing and verifies each
//! supplied sector against the immutable snapshot instead.
//!
//! Finishing is kind-specific and lives with the owning mutation: tracked
//! replacement in `replacement.rs`, durable admission in `admission.rs`.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::checksum::crc_update;
use crate::format7::{MAX_FILE_BYTES, PAYLOAD_SECTOR, Record7};
use crate::{Disk, Error};

use super::payload::{PayloadPlan, run_sector};
use super::{Volume7, WriteIdentity7};

/// Maximum number of concurrently open stages per owner.
pub(super) const STAGES: usize = 2;

/// Source of distinct owner identities for stage tokens. Owned by this module;
/// it is only ever incremented, so no two owner values in one program share an
/// identity and a token from one owner cannot name a stage of another.
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// Exclusive token for one open stage.
///
/// It is neither `Clone` nor `Copy` and borrows nothing, so the owner stays
/// usable between sector writes. It carries the issuing owner's identity and a
/// per-owner nonce, so another `Volume7` value refuses it with
/// [`Error::Invalid`]. After abort, a finished or released stage, a fence or a
/// remount it is stale and every use returns [`Error::Invalid`]. Dropping a
/// token without [`Volume7::abort_stage`] keeps its reservation until
/// [`Volume7::release_stages`], a fence or a remount.
#[derive(Debug)]
#[must_use = "finish or abort the stage to release its reservation"]
pub struct Stage7 {
    owner: u64,
    slot: u8,
    nonce: u64,
}

/// Which mutation a stage will finish as. The kind fixes the preflight and
/// retry rules applied when the stage is opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage7Kind {
    /// Finished by [`Volume7::finish_tracked`] as a direct committed replacement.
    Tracked,
    /// Finished by [`Volume7::finish_admission`] as a durable admission.
    Admission,
}

#[derive(Clone, Copy)]
pub(super) enum StageMode {
    /// New payload written to planned, currently free sectors.
    Fresh(PayloadPlan),
    /// Exact retry verified against a retained immutable snapshot.
    Retry { record: Record7, matches: bool },
}

#[derive(Clone, Copy)]
pub(super) struct StageSlot {
    nonce: u64,
    pub(super) kind: Stage7Kind,
    pub(super) identity: WriteIdentity7,
    pub(super) expected_version: u64,
    pub(super) length: u32,
    pub(super) mode: StageMode,
    written: u32,
    checksum: u32,
}

impl StageSlot {
    /// Whether every logical byte has been written or verified.
    pub(super) fn complete(&self) -> bool {
        self.written == self.length
    }

    /// CRC-32 of the logical bytes supplied so far.
    pub(super) fn payload_crc(&self) -> u32 {
        !self.checksum
    }
}

/// Outcome of kind-specific preflight.
pub(super) enum Opening {
    Fresh(PayloadPlan),
    Retry(Record7),
}

/// Why a sector write stopped, which decides what happens to the stage.
pub(super) enum WriteFault {
    /// An argument refusal before any I/O; the stage is unchanged.
    Refused(Error),
    /// A read or internal refusal without a durable effect; the stage ends.
    Released(Error),
    /// A payload write failed; the owner is already fenced and cleared.
    Fenced,
}

impl WriteFault {
    pub(super) fn error(self) -> Error {
        match self {
            Self::Refused(error) | Self::Released(error) => error,
            Self::Fenced => Error::Uncertain,
        }
    }
}

impl Volume7 {
    /// Open a stage for a file of `length` bytes and return its token.
    ///
    /// Applies the same preflight as [`Volume7::replace_tracked`] or
    /// [`Volume7::prepare_admission`] for `kind`, without I/O. A fresh stage
    /// plans its payload around the volume's free sectors and every other open
    /// stage, and reserves a receipt slot; it returns [`Error::Full`] when
    /// either is unavailable. A matching retained retry record opens a
    /// verifying stage that reserves nothing. Returns [`Error::Busy`] when two
    /// stages are already open or another stage holds the same retry scope.
    pub fn open_stage(
        &mut self,
        identity: WriteIdentity7,
        expected_version: u64,
        length: u32,
        kind: Stage7Kind,
    ) -> Result<Stage7, Error> {
        self.ready()?;
        let index = self
            .stages
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Busy)?;
        let nonce = self.stage_nonce.checked_add(1).ok_or(Error::Exhausted)?;
        let mut slot = self.open_slot(identity, expected_version, length, kind)?;
        if self.stage_owner == 0 {
            let owner = NEXT_OWNER.fetch_add(1, Ordering::Relaxed);
            if owner == 0 {
                return Err(Error::Exhausted);
            }
            self.stage_owner = owner;
        }
        slot.nonce = nonce;
        self.stage_nonce = nonce;
        self.stages[index] = Some(slot);
        Ok(Stage7 {
            owner: self.stage_owner,
            slot: index as u8,
            nonce,
        })
    }

    /// Number of currently open stages, including any whose token was dropped.
    pub fn open_stages(&self) -> usize {
        self.stages.iter().flatten().count()
    }

    /// Release every open stage and its reservations without I/O and without
    /// fencing, so a service can recover from dropped tokens without a
    /// remount. Returns how many stages were released; their tokens are stale.
    pub fn release_stages(&mut self) -> usize {
        let released = self.open_stages();
        self.stages = [None; STAGES];
        released
    }

    /// Supply the next sector of the staged file and return the logical bytes
    /// still expected.
    ///
    /// `bytes.len()` must equal `min(512, remaining)`; the final sector is
    /// zero-padded on disk. A fresh stage issues exactly one payload write to
    /// its next planned sector; a write failure returns [`Error::Uncertain`]
    /// and fences the owner. A retry stage issues exactly one read and records
    /// any byte difference without stopping; a read failure releases the stage
    /// and returns the disk error without fencing. An argument refusal returns
    /// [`Error::Invalid`] and leaves the stage unchanged.
    pub fn stage_write(
        &mut self,
        disk: &mut impl Disk,
        stage: &mut Stage7,
        bytes: &[u8],
    ) -> Result<u32, Error> {
        self.ready()?;
        let index = self.stage_index(stage)?;
        let mut slot = self.stages[index].take().ok_or(Error::Invalid)?;
        match self.write_slot(disk, &mut slot, bytes) {
            Ok(remaining) => {
                self.stages[index] = Some(slot);
                Ok(remaining)
            }
            Err(WriteFault::Refused(error)) => {
                self.stages[index] = Some(slot);
                Err(error)
            }
            Err(fault) => Err(fault.error()),
        }
    }

    /// Release an open stage without I/O. Never fences; payload sectors a
    /// fresh stage already wrote stay free because no metadata names them.
    pub fn abort_stage(&mut self, stage: Stage7) -> Result<(), Error> {
        self.take_stage(&stage).map(|_| ())
    }

    /// Remove a complete stage of `kind` from the owner so it can be
    /// finished. A wrong kind or an incomplete stage is [`Error::Invalid`] and
    /// stays open; otherwise the caller consumes the stage or it ends here.
    pub(super) fn take_finishable(
        &mut self,
        stage: &Stage7,
        kind: Stage7Kind,
    ) -> Result<StageSlot, Error> {
        let index = self.stage_index(stage)?;
        match self.stages[index] {
            Some(slot) if slot.kind == kind && slot.complete() => {
                self.stages[index] = None;
                Ok(slot)
            }
            _ => Err(Error::Invalid),
        }
    }

    fn take_stage(&mut self, stage: &Stage7) -> Result<StageSlot, Error> {
        let index = self.stage_index(stage)?;
        self.stages[index].take().ok_or(Error::Invalid)
    }

    fn stage_index(&self, stage: &Stage7) -> Result<usize, Error> {
        let index = usize::from(stage.slot);
        match self.stages.get(index) {
            Some(Some(slot)) if stage.owner == self.stage_owner && slot.nonce == stage.nonce => {
                Ok(index)
            }
            _ => Err(Error::Invalid),
        }
    }

    /// Kind-specific preflight and reservation planning for a stage that is not
    /// yet registered. Direct replacement uses this with a stack-held slot.
    pub(super) fn open_slot(
        &mut self,
        identity: WriteIdentity7,
        expected_version: u64,
        length: u32,
        kind: Stage7Kind,
    ) -> Result<StageSlot, Error> {
        self.ready()?;
        if length > MAX_FILE_BYTES {
            return Err(Error::Size);
        }
        self.validate_identity(identity)?;
        if self.stage_holds_scope(identity) {
            return Err(Error::Busy);
        }
        let opening = match kind {
            Stage7Kind::Tracked => self.open_tracked(identity, expected_version, length)?,
            Stage7Kind::Admission => self.open_admission(identity, expected_version, length)?,
        };
        let mode = match opening {
            Opening::Fresh(plan) => StageMode::Fresh(plan),
            Opening::Retry(record) => StageMode::Retry {
                record,
                matches: true,
            },
        };
        Ok(StageSlot {
            nonce: 0,
            kind,
            identity,
            expected_version,
            length,
            mode,
            written: 0,
            checksum: !0u32,
        })
    }

    /// Write or verify the next sector of `slot`.
    pub(super) fn write_slot(
        &mut self,
        disk: &mut impl Disk,
        slot: &mut StageSlot,
        bytes: &[u8],
    ) -> Result<u32, WriteFault> {
        let remaining = slot.length - slot.written;
        if remaining == 0 || bytes.len() != remaining.min(512) as usize {
            return Err(WriteFault::Refused(Error::Invalid));
        }
        let index = u64::from(slot.written / 512);
        let mut block = [0u8; 512];
        match &mut slot.mode {
            StageMode::Fresh(plan) => {
                let sector =
                    run_sector(plan.runs(), index).ok_or(WriteFault::Released(Error::Corrupt))?;
                block[..bytes.len()].copy_from_slice(bytes);
                if disk.write(PAYLOAD_SECTOR + sector, &block).is_err() {
                    self.fence_clear();
                    return Err(WriteFault::Fenced);
                }
            }
            StageMode::Retry { record, matches } => {
                let sector =
                    run_sector(record.runs(), index).ok_or(WriteFault::Released(Error::Corrupt))?;
                disk.read(PAYLOAD_SECTOR + sector, &mut block)
                    .map_err(WriteFault::Released)?;
                if block[..bytes.len()] != *bytes {
                    *matches = false;
                }
                // The retry CRC covers the stored snapshot, so corruption is
                // reported even when the supplied bytes also differ.
                crc_update(&mut slot.checksum, &block[..bytes.len()]);
                slot.written += bytes.len() as u32;
                return Ok(slot.length - slot.written);
            }
        }
        crc_update(&mut slot.checksum, bytes);
        slot.written += bytes.len() as u32;
        Ok(slot.length - slot.written)
    }

    /// Whether any open stage holds `identity`'s retry scope.
    pub(super) fn stage_holds_scope(&self, identity: WriteIdentity7) -> bool {
        self.stages.iter().flatten().any(|slot| {
            slot.identity.subject == identity.subject
                && slot.identity.workspace == identity.workspace
                && slot.identity.retry_epoch == identity.retry_epoch
                && slot.identity.retry_key == identity.retry_key
        })
    }

    /// Whether any stage is open, for operations that must not change the
    /// records or epoch a stage depends on.
    pub(super) fn stages_open(&self) -> bool {
        self.stages.iter().any(Option::is_some)
    }

    /// The first free receipt slot that no open fresh stage has reserved.
    pub(super) fn free_record_slot(&self) -> Result<usize, Error> {
        let reserved = self
            .stages
            .iter()
            .flatten()
            .filter(|slot| matches!(slot.mode, StageMode::Fresh(_)))
            .count();
        let free = self
            .records
            .iter()
            .filter(|record| record.is_none())
            .count();
        if free <= reserved {
            return Err(Error::Full);
        }
        self.records
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Full)
    }

    /// The mounted allocation map with every open fresh stage's runs marked
    /// allocated, built in the validator scratch map. Planners use it so a new
    /// payload never overlaps a reservation; `map` itself is not changed.
    pub(super) fn planning_map(&mut self) -> &[u64] {
        self.validation.copy_from_slice(&self.map);
        for slot in self.stages.iter().flatten() {
            if let StageMode::Fresh(plan) = &slot.mode {
                for run in plan.runs() {
                    for sector in run.start..run.end() {
                        self.validation[sector as usize / 64] |= 1u64 << (sector % 64);
                    }
                }
            }
        }
        &self.validation
    }
}
