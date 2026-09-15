// SPDX-License-Identifier: Apache-2.0
//! How this client says what happened, in the five words an owner can observe.
//!
//! The owner reads a child reply through `ACT_STATUS`, which carries words 0..4
//! only. Every layout therefore keeps its meaning inside them, and the module
//! documentation of [`super`] is the contract these functions implement.
use super::state::{Collection, Fault};
use rustic_tasks_client::{Applied, Committed, Error, Note, Pending, Recovery, Report};

/// The durable facts an operation reported while it was still running.
///
/// They are the only way a failed operation can still name a committed effect
/// or the journal key the owner must resolve, so they are kept and reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Trace {
    retained: Option<u64>,
    cleanup: Option<Committed>,
}

impl Report for Trace {
    fn note(&mut self, note: Note) {
        match note {
            Note::Retained { journal } | Note::Unresolved { journal } => {
                self.retained = Some(journal);
            }
            Note::Unverified { journal, .. } => self.retained = Some(journal),
            Note::CleanupIncomplete(committed) => self.cleanup = Some(committed),
            // Ambiguity adds nothing to the reply: the retained key is already
            // known, and no outcome may be inferred from the loss itself.
            _ => {}
        }
    }
}

/// The code for one refusal. `0` is never a refusal.
pub fn code(fault: Fault) -> u64 {
    match fault {
        // File refusals keep the numbering of the file ABI, so a code means the
        // same thing in every reply this application sends.
        Fault::Client(Error::File(error)) => error as u64,
        Fault::Client(Error::Service(code)) => 64u64.saturating_add(code),
        Fault::Client(Error::Document) => 100,
        Fault::Client(Error::Capacity) => 101,
        Fault::Client(Error::Enable) => 102,
        Fault::Client(Error::Journal) => 103,
        Fault::Client(Error::Pending(_)) => 104,
        Fault::Sequence => 105,
        Fault::Memory => 106,
    }
}

/// `[error, cursor, total, phase, 0]` for a step that only moves the client's
/// own state: the edit, the announcement, one chunk, or a forget.
pub fn step(state: &Collection, fault: Option<Fault>) -> [u64; 8] {
    [
        fault.map_or(0, code),
        state.cursor() as u64,
        state.total() as u64,
        state.phase().value(),
        0,
        0,
        0,
        0,
    ]
}

/// `[error, task_id, journal_key, applied, committed_version]`.
///
/// `applied` is meaningful when the effect is proven: `0` for a document that
/// already showed the edit, `1` for a committed and verified effect, and `2`
/// for a committed and verified effect whose intent could not be cleared. That
/// last case keeps the cleanup failure in word 0, because the effect is durable
/// while the record still is not resolved.
pub fn applied(result: Result<Applied, Fault>, trace: &Trace) -> [u64; 8] {
    match result {
        Ok(Applied::Unchanged { task_id, version }) => {
            [0, u64::from(task_id), 0, 0, version.value(), 0, 0, 0]
        }
        Ok(Applied::Committed(committed)) => effect(0, 1, committed),
        Err(fault) => match trace.cleanup {
            Some(committed) => effect(code(fault), 2, committed),
            None => [code(fault), 0, trace.retained.unwrap_or(0), 0, 0, 0, 0, 0],
        },
    }
}

/// `[error, phase, cursor, total, pending_journal_version]`. A non-zero error
/// means the record could not be read, so the pending version is unknown; the
/// phase, cursor and total are this client's own and are always reported.
pub fn status(state: &Collection, pending: Result<Pending, Fault>) -> [u64; 8] {
    let (error, version) = match pending {
        Ok(Pending::Idle) => (0, 0),
        Ok(Pending::Unresolved(version)) => (0, version),
        Err(fault) => (code(fault), 0),
    };
    [
        error,
        state.phase().value(),
        state.cursor() as u64,
        state.total() as u64,
        version,
        0,
        0,
        0,
    ]
}

/// `[error, recovered, journal_key, task_id, version]`. `recovered` is `1` only
/// when a retained intent was matched to a committed effect; an absent record
/// does not prove a forgotten effect never happened.
pub fn recovery(result: Result<Recovery, Fault>, trace: &Trace) -> [u64; 8] {
    match result {
        Ok(Recovery::Absent) => [0; 8],
        Ok(Recovery::Recovered(committed)) => [
            0,
            1,
            committed.journal,
            u64::from(committed.task_id),
            committed.version.value(),
            0,
            0,
            0,
        ],
        Err(fault) => [code(fault), 0, trace.retained.unwrap_or(0), 0, 0, 0, 0, 0],
    }
}

fn effect(error: u64, applied: u64, committed: Committed) -> [u64; 8] {
    [
        error,
        u64::from(committed.task_id),
        committed.journal,
        applied,
        committed.version.value(),
        0,
        0,
        0,
    ]
}

#[cfg(test)]
mod tests {
    use super::super::state::Phase;
    use super::*;
    use rustic_sdk::abi::files::{Error as File, reference::Version};

    fn committed() -> Committed {
        Committed {
            task_id: 8,
            version: Version::new(12).unwrap(),
            journal: 5,
        }
    }

    #[test]
    fn a_refusal_keeps_the_numbering_the_rest_of_the_application_uses() {
        // File refusals are the file ABI's own codes, service refusals are
        // offset past them, and the client's remaining refusals are fixed.
        assert_eq!(code(Fault::Client(Error::File(File::Denied))), 17);
        assert_eq!(code(Fault::Client(Error::File(File::Version))), 13);
        assert_eq!(code(Fault::Client(Error::Service(3))), 67);
        assert_eq!(code(Fault::Client(Error::Document)), 100);
        assert_eq!(code(Fault::Client(Error::Capacity)), 101);
        assert_eq!(code(Fault::Client(Error::Enable)), 102);
        assert_eq!(code(Fault::Client(Error::Journal)), 103);
        assert_eq!(code(Fault::Client(Error::Pending(9))), 104);
        assert_eq!(code(Fault::Sequence), 105);
        assert_eq!(code(Fault::Memory), 106);
        // No file refusal can be mistaken for a service or client refusal.
        assert!(code(Fault::Client(Error::File(File::Unavailable))) < 64);
    }

    #[test]
    fn a_proven_effect_is_reported_even_when_the_operation_failed() {
        let mut trace = Trace::default();
        trace.note(Note::Retained { journal: 5 });
        assert_eq!(
            applied(Ok(Applied::Committed(committed())), &trace),
            [0, 8, 5, 1, 12, 0, 0, 0]
        );
        // An ambiguous failure proves nothing about the effect, but it does name
        // the key the owner must resolve before the next edit.
        assert_eq!(
            applied(Err(Fault::Client(Error::File(File::Uncertain))), &trace),
            [3, 0, 5, 0, 0, 0, 0, 0]
        );
        // A verified effect whose intent could not be cleared reports both.
        trace.note(Note::CleanupIncomplete(committed()));
        assert_eq!(
            applied(Err(Fault::Client(Error::File(File::Io))), &trace),
            [2, 8, 5, 2, 12, 0, 0, 0]
        );
        // A document that already showed the edit submitted nothing, so it has
        // no journal key and no new version.
        assert_eq!(
            applied(
                Ok(Applied::Unchanged {
                    task_id: 7,
                    version: Version::new(4).unwrap()
                }),
                &Trace::default()
            ),
            [0, 7, 0, 0, 4, 0, 0, 0]
        );
    }

    #[test]
    fn an_unreadable_record_still_reports_the_client_s_own_state() {
        let mut storage = [0; 1024];
        let mut state = Collection::new(&mut storage);
        state
            .edit(rustic_tasks_contract::preview::Edit::Done { id: 3 }.words())
            .unwrap();
        assert_eq!(
            status(&state, Ok(Pending::Unresolved(6))),
            [0, Phase::Edit.value(), 0, 0, 6, 0, 0, 0]
        );
        assert_eq!(
            status(&state, Err(Fault::Client(Error::Journal))),
            [103, Phase::Edit.value(), 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            step(&state, None),
            [0, 0, 0, Phase::Edit.value(), 0, 0, 0, 0]
        );
        assert_eq!(
            step(&state, Some(Fault::Sequence)),
            [105, 0, 0, Phase::Edit.value(), 0, 0, 0, 0]
        );
    }

    #[test]
    fn recovery_separates_an_absent_record_from_a_resolved_one() {
        assert_eq!(recovery(Ok(Recovery::Absent), &Trace::default()), [0; 8]);
        assert_eq!(
            recovery(Ok(Recovery::Recovered(committed())), &Trace::default()),
            [0, 1, 5, 8, 12, 0, 0, 0]
        );
        // An unresolved record is not a recovery; the key stays visible so the
        // owner can decide to forget it.
        let mut trace = Trace::default();
        trace.note(Note::Unresolved { journal: 5 });
        assert_eq!(
            recovery(Err(Fault::Client(Error::File(File::NotFound))), &trace),
            [6, 0, 5, 0, 0, 0, 0, 0]
        );
    }
}
