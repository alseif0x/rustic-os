// SPDX-License-Identifier: Apache-2.0
//! What the client holds between owner steps, and which step each phase admits.
//!
//! Collecting a plan is pure: the bytes arrive in fixed messages, the product
//! codec decides what a chunk is, and the owner client decides what a candidate
//! is. Nothing here reads or writes a file, so the order an owner must follow is
//! decided in one place and proved without a guest.
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

/// One candidate under collection, with the edit it claims to implement.
///
/// The single bounded byte buffer lives inside the builder and then inside the
/// candidate; no step keeps a copy of its own.
pub struct Collection {
    edit: Option<Edit>,
    builder: Option<CandidateBuilder>,
    candidate: Option<Candidate>,
    total: usize,
    concluded: bool,
}

impl Default for Collection {
    fn default() -> Self {
        Self::new()
    }
}

impl Collection {
    pub const fn new() -> Self {
        Self {
            edit: None,
            builder: None,
            candidate: None,
            total: 0,
            concluded: false,
        }
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
        let builder = Candidate::begin(total, summary, edit).map_err(Fault::Client)?;
        self.builder = Some(builder);
        self.candidate = None;
        self.total = total;
        self.concluded = false;
        Ok(())
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
            Err(error) => {
                self.clear();
                Err(Fault::Client(error))
            }
        }
    }

    /// The validated plan, when one is ready to apply.
    pub fn candidate(&self) -> Option<&Candidate> {
        self.candidate.as_ref()
    }

    /// Releases the candidate of a concluded apply. An attempt that did not
    /// prove the plan is unpublished keeps it, so the same bytes stay available
    /// to the recovery that must resolve them.
    ///
    /// The stored edit survives, so the owner may announce a fresh plan for the
    /// same command; the released plan's length does not, because no plan is
    /// held any more.
    pub fn conclude(&mut self, release: bool) {
        if release {
            self.candidate = None;
            self.total = 0;
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
        self.edit = None;
        self.builder = None;
        self.candidate = None;
        self.total = 0;
        self.concluded = false;
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

    fn feed(state: &mut Collection, bytes: &[u8]) -> Result<(), Fault> {
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
        let mut state = Collection::new();
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
        let mut state = Collection::new();
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
    }

    #[test]
    fn a_new_announcement_replaces_whatever_was_held() {
        let mut state = Collection::new();
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
        let mut state = Collection::new();
        state.edit(add()).unwrap();
        state.begin(PLAN.len() as u64, summary()).unwrap();
        feed(&mut state, PLAN).unwrap();
        // An attempt that did not prove the plan unpublished keeps it.
        state.conclude(false);
        assert_eq!(state.phase(), Phase::Ready);
        assert!(state.candidate().is_some());
        state.conclude(true);
        assert_eq!(state.phase(), Phase::Finished);
        assert!(state.candidate().is_none());
        assert_eq!((state.cursor(), state.total()), (0, 0));
        // The edit survives a concluded apply, so the same command can be
        // planned again without announcing it twice.
        state.begin(PLAN.len() as u64, summary()).unwrap();
        assert_eq!(state.phase(), Phase::Collecting);
    }

    #[test]
    fn the_summary_and_the_declared_length_must_be_the_planners() {
        let mut state = Collection::new();
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
    }
}
