// SPDX-License-Identifier: Apache-2.0
//! Typed results of owner-client operations. No presentation lives here.
use crate::Error;
use rustic_sdk::abi::files::reference::Version;
use rustic_tasks_contract::{MAX_TASKS, preview::Summary, wire::Row};

/// A completed bounded result from the native application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Listing {
    pub(crate) rows: [Option<Row>; MAX_TASKS],
    pub(crate) count: usize,
    pub(crate) summary: Option<Summary>,
}

impl Listing {
    /// The validated rows, in application order.
    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.rows[..self.count].iter().flatten()
    }
    /// How many rows the application reported.
    pub fn count(&self) -> usize {
        self.count
    }
    /// The preview summary, present only for a preview request.
    pub fn summary(&self) -> Option<Summary> {
        self.summary
    }
}

/// A verified committed effect and the journal version that recorded it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Committed {
    /// The task the edit affected.
    pub task_id: u32,
    /// The committed target version the effect produced.
    pub version: Version,
    /// The journal version that recorded the intent.
    pub journal: u64,
}

/// The result of applying an edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// The candidate equals the committed bytes: nothing was submitted and no
    /// receipt was consumed.
    Unchanged { task_id: u32, version: Version },
    /// The effect is committed and the pinned bytes were read back and compared.
    Committed(Committed),
}

/// The result of a query-only recovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// No intent is retained. This does not prove a forgotten effect never
    /// happened.
    Absent,
    /// A retained intent was matched to a committed effect and cleared.
    Recovered(Committed),
}

/// Whether a stored record leaves the owner free to mutate.
///
/// Exactly one unresolved intent may exist at a time, so a non-empty record
/// blocks every new mutation until recovery or an explicit forget resolves it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    Idle,
    Unresolved(u64),
}

impl Pending {
    /// Derives the decision from the stored record's length and committed
    /// version. An empty record is the idle state; the record is never removed.
    pub fn of(length: usize, version: u64) -> Self {
        if length == 0 {
            Self::Idle
        } else {
            Self::Unresolved(version)
        }
    }
    /// Refuses a new mutation while an intent remains unresolved.
    pub fn idle(self) -> Result<(), Error> {
        match self {
            Self::Idle => Ok(()),
            Self::Unresolved(version) => Err(Error::Pending(version)),
        }
    }
}

/// A durable state change observed while an operation is still running.
///
/// Notes are emitted in the order they happen, before the operation returns, so
/// a caller can report what is already true even when the operation then fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    /// The intent is durable and readback-verified; submission follows.
    Retained { journal: u64 },
    /// The submission failed ambiguously and the intent stays retained.
    Ambiguous,
    /// Recovery could not resolve the intent; the target was not resubmitted.
    Unresolved { journal: u64 },
    /// The effect is committed at this version but its bytes could not be read
    /// back, so the intent stays retained.
    Unverified { journal: u64, version: Version },
    /// The effect is verified but the intent could not be cleared. The caller
    /// still owns reporting the effect, so the note carries it.
    CleanupIncomplete(Committed),
    /// An acceptance cut discarded a real reply without decoding it.
    #[cfg(feature = "tasks-acceptance")]
    ReplyDiscarded,
}

/// Receives notes as they happen. Implementations must not fail.
pub trait Report {
    fn note(&mut self, note: Note);
}

/// Discards notes, for callers that only need the returned outcome.
impl Report for () {
    fn note(&mut self, _note: Note) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_non_empty_record_blocks_the_next_mutation() {
        assert_eq!(Pending::of(0, 12), Pending::Idle);
        assert_eq!(Pending::of(0, 12).idle(), Ok(()));
        // The idle record keeps its object and version; only its length says
        // whether an intent is outstanding.
        assert_eq!(Pending::of(136, 12), Pending::Unresolved(12));
        assert_eq!(Pending::of(136, 12).idle(), Err(Error::Pending(12)));
    }
}
