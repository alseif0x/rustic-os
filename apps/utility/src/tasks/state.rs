// SPDX-License-Identifier: Apache-2.0
//! What the client holds between owner steps, and which step each phase admits.
//!
//! Collecting a plan is pure: the bytes arrive in fixed messages, the product
//! codec decides what a chunk is, and the owner client decides what a candidate
//! is. Nothing here reads or writes a file, allocates or maps a page, so the
//! order an owner must follow is decided in one place and proved without a
//! guest.
//!
//! The bytes themselves live in storage the caller lends: a stack array in these
//! tests, a block of a mapped heap in the running child. A collection therefore
//! borrows its buffer for its whole life, and [`Retained`] is what survives when
//! the caller takes that buffer back — the announced edit and whether the last
//! apply concluded, neither of which lives in the buffer.
use rustic_tasks_client::{Candidate, CandidateBuilder, Error};
use rustic_tasks_contract::{
    candidate,
    preview::{Edit, Summary},
};

/// Why a step did not do what it was asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// A refusal of the owner client, the file service or the document rules.
    Client(Error),
    /// The step is not one the current phase accepts. It changed nothing.
    Sequence,
    /// The client has no storage to hold a plan in: the memory its bytes need
    /// could not be reserved, or was already released. Nothing was retained.
    Memory,
}

/// What the client is holding, as every acknowledgement reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Edit,
    Collecting,
    Ready,
    Finished,
}

impl Phase {
    /// The wire value. It is part of the owner-stepped protocol.
    pub const fn value(self) -> u64 {
        match self {
            Self::Idle => 0,
            Self::Edit => 1,
            Self::Collecting => 2,
            Self::Ready => 3,
            Self::Finished => 4,
        }
    }
}

/// The part of a collection that does not live in its storage.
///
/// A client that releases its buffer keeps the command it was given and the
/// fact that its last apply concluded, because both are answers it still owes
/// the owner; everything else was the plan, and the plan is gone with the bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Retained {
    edit: Option<Edit>,
    concluded: bool,
}

impl Retained {
    /// Whether an edit is stored, so a begin would be admitted.
    pub const fn has_edit(&self) -> bool {
        self.edit.is_some()
    }
}

/// One candidate under collection, with the edit it claims to implement.
///
/// The single bounded byte buffer is the caller's and is lent exactly once: it
/// moves into the builder, from there into the candidate, and back again when a
/// plan is released. No step keeps a copy of its own.
pub struct Collection<'a> {
    edit: Option<Edit>,
    /// The lent buffer while neither the builder nor the candidate holds it.
    storage: Option<&'a mut [u8]>,
    builder: Option<CandidateBuilder<'a>>,
    candidate: Option<Candidate<'a>>,
    /// Bytes of the lent buffer, `0` when nothing was lent at all.
    capacity: usize,
    total: usize,
    concluded: bool,
}

impl<'a> Collection<'a> {
    /// A collection that assembles plans in `storage` and holds nothing yet.
    pub const fn new(storage: &'a mut [u8]) -> Self {
        Self {
            edit: None,
            capacity: storage.len(),
            storage: Some(storage),
            builder: None,
            candidate: None,
            total: 0,
            concluded: false,
        }
    }

    /// A collection over fresh storage that adopts what its predecessor kept
    /// when the buffer they share was released.
    pub const fn resume(storage: &'a mut [u8], retained: Retained) -> Self {
        let mut value = Self::new(storage);
        value.edit = retained.edit;
        value.concluded = retained.concluded;
        value
    }

    /// A collection with no storage at all. It reports what it holds and
    /// refuses every step that would need bytes, without losing the edit.
    pub const fn detached(retained: Retained) -> Self {
        Self {
            edit: retained.edit,
            storage: None,
            builder: None,
            candidate: None,
            capacity: 0,
            total: 0,
            concluded: retained.concluded,
        }
    }

    /// What survives the release of the storage this collection borrows.
    pub const fn retained(&self) -> Retained {
        Retained {
            edit: self.edit,
            concluded: self.concluded,
        }
    }

    /// Whether the client still needs the storage it was lent.
    ///
    /// The buffer is for one hand-off: an announced edit is its first half, so
    /// it is needed from the edit until the plan it announced is released again.
    /// An idle client and a client whose last apply concluded hold nothing the
    /// bytes are for.
    pub const fn holds_storage(&self) -> bool {
        !matches!(self.phase(), Phase::Idle | Phase::Finished)
    }

    /// Stores the edit a later plan must implement. Any bytes collected for an
    /// earlier edit are dropped: they prove nothing about this one.
    pub fn edit(&mut self, words: [u64; 6]) -> Result<(), Fault> {
        let edit = Edit::decode(words).ok_or(Fault::Client(Error::Document))?;
        self.clear();
        self.edit = Some(edit);
        Ok(())
    }

    /// Begins collecting `total` bytes for the stored edit and the summary the
    /// planner reported. A begin always restarts collection, so a lost or
    /// duplicated chunk is recovered by replaying the whole plan.
    pub fn begin(&mut self, total: u64, summary: [u64; 4]) -> Result<(), Fault> {
        let edit = self.edit.ok_or(Fault::Sequence)?;
        let summary = Summary::decode([summary[0], summary[1], summary[2], summary[3], 0, 0, 0])
            .ok_or(Fault::Client(Error::Document))?;
        let total = usize::try_from(total).map_err(|_| Fault::Client(Error::Document))?;
        if self.capacity == 0 {
            return Err(Fault::Memory);
        }
        // Asked before the buffer is taken back from whatever holds it, so a
        // declaration this client cannot collect leaves the plan it already has
        // exactly where it was.
        CandidateBuilder::accepts(self.capacity, total, summary, edit).map_err(Fault::Client)?;
        self.reclaim();
        let storage = self.storage.take().ok_or(Fault::Memory)?;
        match CandidateBuilder::new(storage, total, summary, edit) {
            Ok(builder) => {
                self.builder = Some(builder);
                self.total = total;
                self.concluded = false;
                Ok(())
            }
            // The declaration was accepted above, so this cannot happen; the
            // buffer still goes back to its owner rather than being lost.
            Err(refused) => {
                self.storage = Some(refused.storage);
                Err(Fault::Client(refused.error))
            }
        }
    }

    /// Appends the next chunk at the implicit cursor, and validates the whole
    /// plan once the declared total has arrived.
    ///
    /// The offset is the order of arrival: a chunk that does not fit is refused
    /// without moving the cursor, and bytes that arrive in the wrong order fail
    /// the document checks instead of being submitted.
    pub fn chunk(&mut self, length: u64, bytes: [u64; 4]) -> Result<(), Fault> {
        let chunk =
            candidate::decode_owner([0, 0, length, length, bytes[0], bytes[1], bytes[2], bytes[3]])
                .ok_or(Fault::Client(Error::Document))?;
        let builder = self.builder.as_mut().ok_or(Fault::Sequence)?;
        builder.push(chunk.bytes()).map_err(Fault::Client)?;
        if !builder.complete() {
            return Ok(());
        }
        // `finish` consumes the builder, so a plan that fails validation leaves
        // nothing behind to submit or to mistake for a later plan. The builder
        // was just borrowed, so the take never fails.
        let Some(builder) = self.builder.take() else {
            return Err(Fault::Sequence);
        };
        match builder.finish() {
            Ok(candidate) => {
                self.candidate = Some(candidate);
                Ok(())
            }
            Err(refused) => {
                self.storage = Some(refused.storage);
                self.clear();
                Err(Fault::Client(refused.error))
            }
        }
    }

    /// The validated plan, when one is ready to apply.
    pub fn candidate(&self) -> Option<&Candidate<'a>> {
        self.candidate.as_ref()
    }

    /// Releases the candidate of a concluded apply. An attempt that did not
    /// prove the plan is unpublished keeps it, so the same bytes stay available
    /// to the recovery that must resolve them.
    ///
    /// The stored edit survives, so the owner may announce a fresh plan for the
    /// same command; the released plan's length does not, because no plan is
    /// held any more, and the buffer goes back to the client, which no longer
    /// needs it.
    pub fn conclude(&mut self, release: bool) {
        if release {
            self.reclaim();
            self.concluded = true;
        }
    }

    pub const fn phase(&self) -> Phase {
        if self.candidate.is_some() {
            Phase::Ready
        } else if self.builder.is_some() {
            Phase::Collecting
        } else if self.concluded {
            Phase::Finished
        } else if self.edit.is_some() {
            Phase::Edit
        } else {
            Phase::Idle
        }
    }

    /// How many of the declared bytes are held.
    pub fn cursor(&self) -> usize {
        match (&self.builder, &self.candidate) {
            (Some(builder), _) => self.total - builder.remaining(),
            (None, Some(candidate)) => candidate.bytes().len(),
            (None, None) => 0,
        }
    }

    /// How many bytes the plan under collection declared.
    pub const fn total(&self) -> usize {
        self.total
    }

    fn clear(&mut self) {
        self.reclaim();
        self.edit = None;
        self.concluded = false;
    }

    /// Takes the one buffer back from whatever holds it, dropping the plan or
    /// the partial stream that was in it.
    fn reclaim(&mut self) {
        if let Some(builder) = self.builder.take() {
            self.storage = Some(builder.release());
        }
        if let Some(candidate) = self.candidate.take() {
            self.storage = Some(candidate.release());
        }
        self.total = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: &[u8] = b"rustic-tasks-v1\n7\tdone\tFirst\n8\topen\tSecond\n";

    fn add() -> [u64; 6] {
        Edit::add(b"Second").unwrap().words()
    }

    fn summary() -> [u64; 4] {
        [2, 9, 8, 1]
    }

    fn chunk(bytes: &[u8], offset: usize) -> (u64, [u64; 4]) {
        let words = candidate::owner_words(&candidate::chunk(bytes, offset).unwrap());
        (words[3], [words[4], words[5], words[6], words[7]])
    }

    fn feed(state: &mut Collection<'_>, bytes: &[u8]) -> Result<(), Fault> {
        let mut offset = 0;
        while offset < bytes.len() {
            let (length, words) = chunk(bytes, offset);
            state.chunk(length, words)?;
            offset += length as usize;
        }
        Ok(())
    }

    #[test]
    fn bytes_are_collected_only_for_an_announced_edit() {
        let mut storage = [0; 1024];
        let mut state = Collection::new(&mut storage);
        assert_eq!(state.phase(), Phase::Idle);
        // Nothing may be collected before the client knows which edit the bytes
        // are supposed to implement, or how many of them to expect.
        assert_eq!(
            state.begin(PLAN.len() as u64, summary()),
            Err(Fault::Sequence)
        );
        let (length, words) = chunk(PLAN, 0);
        assert_eq!(state.chunk(length, words), Err(Fault::Sequence));
        state.edit(add()).unwrap();
        assert_eq!(state.phase(), Phase::Edit);
        assert_eq!(state.chunk(length, words), Err(Fault::Sequence));
        state.begin(PLAN.len() as u64, summary()).unwrap();
        assert_eq!(state.phase(), Phase::Collecting);
        assert_eq!((state.cursor(), state.total()), (0, PLAN.len()));
        feed(&mut state, PLAN).unwrap();
        assert_eq!(state.phase(), Phase::Ready);
        assert_eq!((state.cursor(), state.total()), (PLAN.len(), PLAN.len()));
        assert_eq!(state.candidate().unwrap().bytes(), PLAN);
    }

    #[test]
    fn a_plan_that_arrives_out_of_order_is_never_applied() {
        let mut storage = [0; 1024];
        let mut state = Collection::new(&mut storage);
        state.edit(add()).unwrap();
        state.begin(PLAN.len() as u64, summary()).unwrap();
        let (length, words) = chunk(PLAN, 0);
        state.chunk(length, words).unwrap();
        // The declared total bounds the stream, so a repeated chunk does not fit
        // and the cursor does not move.
        assert_eq!(
            state.chunk(length, words),
            Err(Fault::Client(Error::Document))
        );
        assert_eq!(state.cursor(), 32);
        // Bytes that complete the stream in the wrong order produce a document
        // that is not this plan, and leave nothing behind to submit.
        let (length, words) = chunk(&PLAN[..PLAN.len() - 32], 0);
        assert_eq!(
            state.chunk(length, words),
            Err(Fault::Client(Error::Document))
        );
        assert_eq!(state.phase(), Phase::Idle);
        assert!(state.candidate().is_none());
        assert_eq!((state.cursor(), state.total()), (0, 0));
        // The refused plan handed the buffer back, so the next one is collected
        // into the same bytes.
        state.edit(add()).unwrap();
        state.begin(PLAN.len() as u64, summary()).unwrap();
        feed(&mut state, PLAN).unwrap();
        assert_eq!(state.phase(), Phase::Ready);
    }

    #[test]
    fn a_new_announcement_replaces_whatever_was_held() {
        let mut storage = [0; 1024];
        let mut state = Collection::new(&mut storage);
        state.edit(add()).unwrap();
        state.begin(PLAN.len() as u64, summary()).unwrap();
        feed(&mut state, PLAN).unwrap();
        // A begin restarts collection even when a candidate is ready, so a plan
        // is never assembled from two different announcements.
        state.begin(PLAN.len() as u64, summary()).unwrap();
        assert_eq!(state.phase(), Phase::Collecting);
        assert_eq!(state.cursor(), 0);
        feed(&mut state, PLAN).unwrap();
        // A new edit drops a ready candidate: it proves nothing about this edit.
        state.edit(Edit::Done { id: 7 }.words()).unwrap();
        assert_eq!(state.phase(), Phase::Edit);
        assert!(state.candidate().is_none());
        // And an edit that is not the planner's canonical encoding is refused.
        assert_eq!(
            state.edit([1, 1, 0, 0, 0, 0]),
            Err(Fault::Client(Error::Document))
        );
    }

    #[test]
    fn only_a_concluded_apply_releases_the_candidate() {
        let mut storage = [0; 1024];
        let mut state = Collection::new(&mut storage);
        state.edit(add()).unwrap();
        state.begin(PLAN.len() as u64, summary()).unwrap();
        feed(&mut state, PLAN).unwrap();
        // An attempt that did not prove the plan unpublished keeps it, and keeps
        // the storage it lives in.
        state.conclude(false);
        assert_eq!(state.phase(), Phase::Ready);
        assert!(state.candidate().is_some());
        assert!(state.holds_storage());
        state.conclude(true);
        assert_eq!(state.phase(), Phase::Finished);
        assert!(state.candidate().is_none());
        assert_eq!((state.cursor(), state.total()), (0, 0));
        // A concluded apply needs no bytes any more, so its client may hand the
        // buffer back.
        assert!(!state.holds_storage());
        // The edit survives a concluded apply, so the same command can be
        // planned again without announcing it twice.
        state.begin(PLAN.len() as u64, summary()).unwrap();
        assert_eq!(state.phase(), Phase::Collecting);
        assert!(state.holds_storage());
    }

    #[test]
    fn the_summary_and_the_declared_length_must_be_the_planners() {
        let mut storage = [0; 1024];
        let mut state = Collection::new(&mut storage);
        state.edit(add()).unwrap();
        // An unversioned or unidentified summary, and a length no bounded
        // document can have, are refused before any byte is accepted.
        for summary in [[2, 0, 8, 1], [2, 9, 0, 1], [2, 9, 8, 2]] {
            assert_eq!(
                state.begin(PLAN.len() as u64, summary),
                Err(Fault::Client(Error::Document))
            );
        }
        assert_eq!(
            state.begin(0, summary()),
            Err(Fault::Client(Error::Document))
        );
        assert_eq!(
            state.begin(u64::from(u32::MAX), summary()),
            Err(Fault::Client(Error::Document))
        );
        assert_eq!(state.phase(), Phase::Edit);
        // A refused declaration never takes the buffer away from the plan that
        // is already held.
        state.begin(PLAN.len() as u64, summary()).unwrap();
        feed(&mut state, PLAN).unwrap();
        assert_eq!(
            state.begin(PLAN.len() as u64, [2, 0, 8, 1]),
            Err(Fault::Client(Error::Document))
        );
        assert_eq!(state.phase(), Phase::Ready);
        assert_eq!(state.candidate().unwrap().bytes(), PLAN);
    }

    #[test]
    fn a_client_without_storage_keeps_its_answers_but_collects_nothing() {
        // What a released buffer leaves behind: the command and the concluded
        // apply are still the client's to report.
        let mut storage = [0; 1024];
        let mut state = Collection::new(&mut storage);
        state.edit(add()).unwrap();
        state.begin(PLAN.len() as u64, summary()).unwrap();
        feed(&mut state, PLAN).unwrap();
        state.conclude(true);
        let retained = state.retained();
        let mut detached = Collection::detached(retained);
        assert_eq!(detached.phase(), Phase::Finished);
        assert_eq!((detached.cursor(), detached.total()), (0, 0));
        // Without bytes to collect into, an announcement is refused as a memory
        // refusal and not as a document that failed the planner's rules.
        assert_eq!(
            detached.begin(PLAN.len() as u64, summary()),
            Err(Fault::Memory)
        );
        assert_eq!(detached.phase(), Phase::Finished);
        // A fresh buffer resumes exactly what the released one left.
        let mut other = [0; 1024];
        let mut state = Collection::resume(&mut other, retained);
        assert_eq!(state.phase(), Phase::Finished);
        state.begin(PLAN.len() as u64, summary()).unwrap();
        feed(&mut state, PLAN).unwrap();
        assert_eq!(state.candidate().unwrap().bytes(), PLAN);
        assert_eq!(
            state.retained(),
            Retained {
                edit: retained.edit,
                concluded: false
            }
        );
    }
}
