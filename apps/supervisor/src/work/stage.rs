// SPDX-License-Identifier: Apache-2.0
//! One storage-sourced dormant image: a pinned V7 pair fed into kernel staging.
//!
//! The job reads the manifest, then every ELF range in order through the
//! supervisor's own read-only owner client, and forwards each verified range into
//! the kernel's single image transaction. `rustic_supervisor::image_pair` decides which
//! observations belong to the pinned pair; this module owns the reader, the
//! transaction generation and their cleanup. A refusal aborts an open
//! transaction and drains an abandoned read before the job reports it, so the
//! owner client and the kernel stage are both reusable afterwards.
//!
//! The resulting child is dormant: nothing here starts it or gives it an
//! endpoint, and the manifest's executable name is not bound to the storage node
//! that supplied the bytes. The job keeps the admitted manifest's facts with the
//! child so that `super::start` can apply the storage launch policy later.
use super::super::services::{FileProfile, State};
use super::Task;
use rustic_sdk::{
    abi::{
        application,
        files::{Error as FileError, read::MAX_RANGE},
        runtime as k, supervisor as s,
    },
    files::{RangeProgress, RangeRead, VerifiedRange},
    runtime,
};
use rustic_supervisor::{
    image_pair::{Pin, STORAGE_FEATURES, Transfer, file_refusal, kernel_refusal},
    storage_launch::Facts,
};

/// Job budget in 100 Hz PIT ticks, sized from measurement rather than the
/// ordinary 1,000-tick owner deadline. Staging the shipped 324,528-byte
/// `file-server.elf` (317 ranges plus the manifest, about 8,600 service
/// exchanges) took 1,697-4,786 ticks in eight stages across four
/// `tools/v7_read_test.py` guest runs under QEMU TCG on the reference machine.
/// The cost is dominated by the service's per-chunk sector reads, not by this
/// job. The budget is about three times the slowest sample. The job
/// never blocks owner control, but it holds the single owner job slot until it
/// ends; an expired job reports a timeout, aborts its transaction and leaves the
/// supervisor degraded until `restart files`.
pub(super) const BUDGET_TICKS: u64 = 15_000;

/// Reader steps per supervisor loop turn. A turn after a reply needs at most
/// three: accept the reply (or finish a range and open the next), send the next
/// request, and one receive that normally finds nothing. The supervisor then waits
/// on the owner endpoint instead of spinning.
const STEPS_PER_TURN: usize = 3;

/// `job-status` phase once the kernel transaction is open; before it, the job
/// reports the ordinary starting phase `1` while it reads the manifest and the
/// first ELF range.
const COPYING: u64 = 2;

/// The single staged child the owner may inspect, start, kill and reap. Until
/// it is started it holds no authority: the supervisor issued it no endpoint,
/// grant or start.
#[derive(Clone, Copy)]
pub(in super::super) struct Staged {
    pub pid: u64,
    pub generation: u64,
    /// Ticks from job start until the kernel returned the dormant child.
    pub ticks: u64,
    pub bytes: u64,
    pub ranges: u32,
    /// The admitted manifest, as the storage launch policy reads it.
    pub facts: Facts,
    /// Present once the child was started under the control-only topology.
    pub started: Option<super::start::Started>,
}

enum Phase {
    Manifest,
    Elf(Transfer),
    /// The pair was refused; an abandoned read is drained before the job ends.
    Refusing(u64),
}

enum Turn {
    Continue,
    Yield,
    Done([u64; 8]),
}

pub(super) struct Stage {
    pin: Pin,
    phase: Phase,
    read: Option<RangeRead>,
    manifest: [u8; application::SIZE],
    /// Open kernel transaction, zero when none is owned by this job.
    generation: u64,
    started: u64,
}

impl Stage {
    fn new(pin: Pin) -> Self {
        Self {
            pin,
            phase: Phase::Manifest,
            read: None,
            manifest: [0; application::SIZE],
            generation: 0,
            started: runtime::clock(),
        }
    }

    /// Whether a request of this job may be waiting on the owner endpoint.
    pub(super) fn reading(&self) -> bool {
        self.read.is_some() || matches!(self.phase, Phase::Refusing(_))
    }

    pub(super) fn poll(&mut self, state: &mut State) -> Result<Option<[u64; 8]>, u64> {
        for _ in 0..STEPS_PER_TURN {
            if let Phase::Refusing(code) = self.phase {
                return drain(state, code);
            }
            match self.advance(state) {
                Ok(Turn::Continue) => {}
                Ok(Turn::Yield) => return Ok(None),
                Ok(Turn::Done(words)) => return Ok(Some(words)),
                Err(code) => self.refuse(code),
            }
        }
        Ok(None)
    }

    /// Release everything this job still holds when it will not be polled again.
    pub(super) fn cancel(&mut self, state: &mut State) {
        self.abort();
        if self.read.take().is_some() || matches!(self.phase, Phase::Refusing(_)) {
            // One bounded attempt. A reply still in flight keeps the owner client
            // reserved; the next stage job drains it before reading.
            if let Err(FileError::Protocol) = state.owner.drain_abandoned_read_range() {
                state.degraded = true;
            }
        }
    }

    fn advance(&mut self, state: &mut State) -> Result<Turn, u64> {
        let Some(read) = self.read.as_mut() else {
            return self.open(state);
        };
        match read.poll(&mut state.owner) {
            Ok(RangeProgress::Pending) => Ok(Turn::Continue),
            Ok(RangeProgress::Complete) => {
                let verified = self
                    .read
                    .take()
                    .ok_or(4u64)?
                    .finish()
                    .map_err(file_refusal)?;
                self.accept(&verified)?;
                if self.generation != 0 {
                    // Owner-visible progress: the kernel transaction is open.
                    state.work.phase = COPYING;
                }
                Ok(Turn::Continue)
            }
            // The send was not admitted; nothing was submitted, so try next turn.
            Err(FileError::Busy) => Ok(Turn::Yield),
            Err(error) => Err(file_refusal(error)),
        }
    }

    /// Start the next pinned read, or commit once the whole image was copied.
    fn open(&mut self, state: &mut State) -> Result<Turn, u64> {
        let request = match &self.phase {
            Phase::Manifest => self.pin.manifest_request(),
            Phase::Elf(transfer) => match transfer.request() {
                Some(request) => request,
                None => return self.commit(state, *transfer),
            },
            Phase::Refusing(_) => return Err(4),
        };
        match state.owner.begin_read_range(request) {
            Ok(read) => {
                self.read = Some(read);
                Ok(Turn::Continue)
            }
            // A previous job may have left its abandoned read in flight.
            Err(FileError::Busy) => match state.owner.drain_abandoned_read_range() {
                Ok(true) => Ok(Turn::Continue),
                Ok(false) => Ok(Turn::Yield),
                Err(FileError::Unavailable) => Err(file_refusal(FileError::Busy)),
                Err(error) => Err(file_refusal(error)),
            },
            Err(error) => Err(file_refusal(error)),
        }
    }

    fn accept(&mut self, verified: &VerifiedRange) -> Result<(), u64> {
        let info = verified.info();
        let transfer = match &mut self.phase {
            Phase::Manifest => {
                let epoch = self.pin.accept_manifest(&info)?;
                self.manifest.copy_from_slice(verified.bytes());
                self.phase = Phase::Elf(Transfer::new(self.pin, epoch));
                return Ok(());
            }
            Phase::Elf(transfer) => transfer,
            Phase::Refusing(_) => return Err(4),
        };
        let accepted = transfer.accept(&info)?;
        if let Some(length) = accepted.begin {
            // The kernel copies and validates the manifest against this policy
            // before it allocates the image buffer.
            let r = control([
                k::STAGE_BEGIN,
                length,
                self.manifest.as_ptr() as u64,
                STORAGE_FEATURES,
                0,
                0,
                0,
                0,
            ])?;
            if r[0] == 0 {
                return Err(4);
            }
            self.generation = r[0];
            if r[1] < MAX_RANGE as u64 {
                return Err(4);
            }
        }
        let bytes = verified.bytes();
        let r = control([
            k::STAGE_COPY,
            self.generation,
            accepted.offset,
            bytes.as_ptr() as u64,
            bytes.len() as u64,
            0,
            0,
            0,
        ])?;
        if r[0] != accepted.offset + accepted.length as u64 {
            return Err(4);
        }
        Ok(())
    }

    fn commit(&mut self, state: &mut State, transfer: Transfer) -> Result<Turn, u64> {
        let generation = core::mem::take(&mut self.generation);
        // The kernel consumes the transaction whether or not it admits the image.
        let r = control([k::STAGE_COMMIT, generation, 0, 0, 0, 0, 0, 0])?;
        let pid = r[0];
        if pid == 0 {
            return Err(4);
        }
        if r[1] != generation {
            // A child answered for another transaction is not this pair's
            // result; it is withdrawn instead of being tracked or reported.
            super::super::services::stop(pid);
            return Err(4);
        }
        // The kernel admitted exactly these manifest bytes, so they parse; a
        // child whose facts cannot be kept is withdrawn rather than tracked.
        let Some(facts) = Facts::parse(&self.manifest) else {
            super::super::services::stop(pid);
            return Err(4);
        };
        state.staged = Some(Staged {
            pid,
            generation,
            ticks: runtime::clock().saturating_sub(self.started),
            bytes: transfer.size().unwrap_or(0),
            ranges: transfer.ranges(),
            facts,
            started: None,
        });
        Ok(Turn::Done([
            0,
            pid,
            self.pin.elf_version().value(),
            self.pin.manifest_version().value(),
            0,
            0,
            0,
            0,
        ]))
    }

    fn refuse(&mut self, code: u64) {
        self.abort();
        // A completed or failed read has already released the client; one
        // dropped mid-flight is drained before the refusal is reported.
        self.read = None;
        self.phase = Phase::Refusing(code);
    }

    fn abort(&mut self) {
        let generation = core::mem::take(&mut self.generation);
        if generation != 0 {
            // A refused copy or commit already cleared the stage; aborting a
            // cleared generation is refused harmlessly.
            let _ = runtime::control([k::STAGE_ABORT, generation, 0, 0, 0, 0, 0, 0]);
        }
    }
}

fn control(words: [u64; 8]) -> Result<[u64; 8], u64> {
    runtime::control(words).map_err(kernel_refusal)
}

fn drain(state: &mut State, code: u64) -> Result<Option<[u64; 8]>, u64> {
    match state.owner.drain_abandoned_read_range() {
        Ok(true) | Err(FileError::Unavailable) => Err(code),
        Ok(false) => Ok(None),
        Err(FileError::Protocol) => {
            state.degraded = true;
            Err(code)
        }
        Err(_) => Err(code),
    }
}

impl State {
    pub(in super::super) fn stage_v7(&mut self, words: [u64; 8]) -> Result<[u64; 8], u64> {
        if self.profile != FileProfile::V7 {
            return Err(1);
        }
        let pin = Pin::decode(words)?;
        if self.files == 0
            || self.degraded
            || self.stopping
            || self.staged.is_some()
            || !self.work.can_start()
        {
            return Err(3);
        }
        self.work
            .start_budget(s::STAGE_V7, Task::StageV7(Stage::new(pin)), BUDGET_TICKS)
    }

    /// Owner kill of the staged child. The record stays until it is reaped.
    pub(in super::super) fn staged_pid(&self, pid: u64) -> bool {
        self.staged.is_some_and(|staged| staged.pid == pid)
    }

    pub(in super::super) fn staged_facts(&self, pid: u64) -> Option<[u64; 8]> {
        let staged = self.staged.filter(|staged| staged.pid == pid)?;
        if let Some(started) = staged.started {
            // Words 5..7 carry the child's report, as for a utility child.
            return Some(started.facts(staged.generation));
        }
        // No scope, rights or expiry: the child was granted nothing.
        Some([
            0,
            0,
            0,
            staged.generation,
            0,
            staged.ticks,
            staged.bytes,
            staged.ranges.into(),
        ])
    }

    pub(in super::super) fn reap_staged(&mut self, pid: u64) -> Result<[u64; 8], u64> {
        if !self.staged_pid(pid) {
            return Err(2);
        }
        let r = runtime::control([k::REAP, pid, 0, 0, 0, 0, 0, 0]).map_err(|_| 3u64)?;
        if let Some(started) = self.staged.and_then(|staged| staged.started) {
            started.close();
        }
        self.staged = None;
        Ok([0, r[0], r[1], 0, 0, 0, 0, 0])
    }
}
