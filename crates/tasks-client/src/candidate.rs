// SPDX-License-Identifier: Apache-2.0
//! One planned replacement document, validated before anything retains it.
//!
//! Planning happens in the isolated native application; this type is only the
//! bounded result a client may carry. The checks live here so that a client
//! which collected the plan itself and a client which received it in chunks from
//! another process reach the submission path under identical rules: the bytes
//! must parse as a task document, and that document must agree with the summary
//! and the command the plan claims to implement.
use crate::Error;
use rustic_tasks_contract::{
    Document, MAX_BYTES, State,
    preview::{Edit, Summary},
};

/// An immutable candidate document with the command and summary that identify
/// the edit it implements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    bytes: [u8; MAX_BYTES],
    length: usize,
    summary: Summary,
    edit: Edit,
}

/// Assembles one candidate from the chunks of an owner-stepped transport.
///
/// The declared total is fixed at the start, so a truncated or overlong stream
/// is refused as it arrives rather than after a partial document is parsed.
pub struct CandidateBuilder {
    bytes: [u8; MAX_BYTES],
    length: usize,
    total: usize,
    summary: Summary,
    edit: Edit,
}

impl Candidate {
    /// Accepts complete planned bytes.
    ///
    /// The summary and command are required because the intent records them: an
    /// edit is recovered by the command and task it named, so a document that
    /// does not show that command's effect is not this plan.
    pub fn new(bytes: &[u8], summary: Summary, edit: Edit) -> Result<Self, Error> {
        canonical(summary, edit)?;
        if bytes.len() > MAX_BYTES {
            return Err(Error::Document);
        }
        agrees(bytes, summary, edit)?;
        let mut value = Self {
            bytes: [0; MAX_BYTES],
            length: bytes.len(),
            summary,
            edit,
        };
        value.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(value)
    }

    /// Begins assembling a candidate of exactly `total` bytes.
    pub fn begin(total: usize, summary: Summary, edit: Edit) -> Result<CandidateBuilder, Error> {
        canonical(summary, edit)?;
        if total == 0 || total > MAX_BYTES {
            return Err(Error::Document);
        }
        Ok(CandidateBuilder {
            bytes: [0; MAX_BYTES],
            length: 0,
            total,
            summary,
            edit,
        })
    }

    /// The planned bytes, exactly as the application produced them.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }

    /// The summary of the edit that produced these bytes.
    pub const fn summary(&self) -> Summary {
        self.summary
    }

    /// The command this plan implements, recorded with the intent.
    pub const fn edit(&self) -> Edit {
        self.edit
    }
}

impl CandidateBuilder {
    /// Appends the next chunk at the implicit cursor. Bytes beyond the declared
    /// total, and an empty chunk that would never finish the stream, are refused.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let end = self
            .length
            .checked_add(bytes.len())
            .ok_or(Error::Document)?;
        if bytes.is_empty() || end > self.total {
            return Err(Error::Document);
        }
        self.bytes[self.length..end].copy_from_slice(bytes);
        self.length = end;
        Ok(())
    }

    /// How many bytes are still missing before the candidate can be finished.
    pub const fn remaining(&self) -> usize {
        self.total - self.length
    }

    /// Whether the declared total has arrived.
    pub const fn complete(&self) -> bool {
        self.length == self.total
    }

    /// Validates the assembled bytes. An incomplete stream is never a candidate.
    pub fn finish(self) -> Result<Candidate, Error> {
        if !self.complete() {
            return Err(Error::Document);
        }
        Candidate::new(&self.bytes[..self.length], self.summary, self.edit)
    }
}

/// A summary or command that does not survive its own encoding was not produced
/// by the application: both travel in fixed words that validate on decode.
fn canonical(summary: Summary, edit: Edit) -> Result<(), Error> {
    if Summary::decode(summary.words()) != Some(summary) || Edit::decode(edit.words()) != Some(edit)
    {
        return Err(Error::Document);
    }
    Ok(())
}

/// Checks the parsed document against the summary and the command, the way the
/// collecting transport checks it against the rows the application reported.
fn agrees(bytes: &[u8], summary: Summary, edit: Edit) -> Result<(), Error> {
    let document = Document::parse(bytes).map_err(|_| Error::Document)?;
    if document.len() != summary.count as usize {
        return Err(Error::Document);
    }
    let mut affected = None;
    let mut greatest = 0;
    for index in 0..document.len() {
        let task = document.get(index).ok_or(Error::Document)?;
        greatest = greatest.max(task.id);
        if task.id == summary.task_id {
            affected = Some((index, task));
        }
    }
    let (index, task) = affected.ok_or(Error::Document)?;
    match edit {
        // The named task is done in the result, whether this plan completed it
        // or repeated an already completed task.
        Edit::Done { id } => {
            if id != summary.task_id || task.state != State::Done {
                return Err(Error::Document);
            }
        }
        // An append always changes the document and puts one open task carrying
        // the requested title last, under the successor of the greatest ID.
        Edit::Add { title, length } => {
            if !summary.changed
                || index + 1 != document.len()
                || task.id != greatest
                || task.state != State::Open
                || task.title() != &title[..usize::from(length)]
            {
                return Err(Error::Document);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDED: &[u8] = b"rustic-tasks-v1\n7\tdone\tFirst\n8\topen\tSecond\n";
    const DONE: &[u8] = b"rustic-tasks-v1\n7\tdone\tFirst\n";

    fn summary(count: u32, task_id: u32, changed: bool) -> Summary {
        Summary {
            count,
            version: 7,
            task_id,
            changed,
        }
    }

    fn add() -> Edit {
        Edit::add(b"Second").unwrap()
    }

    #[test]
    fn a_candidate_must_show_the_effect_of_the_command_it_claims() {
        let accepted = Candidate::new(ADDED, summary(2, 8, true), add()).unwrap();
        assert_eq!(accepted.bytes(), ADDED);
        assert_eq!(accepted.summary(), summary(2, 8, true));
        assert_eq!(accepted.edit(), add());
        Candidate::new(DONE, summary(1, 7, true), Edit::Done { id: 7 }).unwrap();
        // Repeating a completed task changes nothing but still shows it done.
        Candidate::new(DONE, summary(1, 7, false), Edit::Done { id: 7 }).unwrap();
        // A task the document still leaves open does not prove this edit.
        assert_eq!(
            Candidate::new(
                b"rustic-tasks-v1\n7\topen\tFirst\n",
                summary(1, 7, true),
                Edit::Done { id: 7 }
            ),
            Err(Error::Document)
        );
        // An append must carry the requested title, last, under a new ID.
        assert_eq!(
            Candidate::new(ADDED, summary(2, 8, true), Edit::add(b"Other").unwrap()),
            Err(Error::Document)
        );
        assert_eq!(
            Candidate::new(
                b"rustic-tasks-v1\n8\topen\tSecond\n7\tdone\tFirst\n",
                summary(2, 8, true),
                add()
            ),
            Err(Error::Document)
        );
    }

    #[test]
    fn a_summary_that_disagrees_with_the_document_is_refused() {
        // Count, affected task and canonical encoding are all checked: a caller
        // can build a summary the application never could.
        assert_eq!(
            Candidate::new(ADDED, summary(3, 8, true), add()),
            Err(Error::Document)
        );
        assert_eq!(
            Candidate::new(ADDED, summary(2, 9, true), add()),
            Err(Error::Document)
        );
        assert_eq!(
            Candidate::new(ADDED, summary(2, 8, false), add()),
            Err(Error::Document)
        );
        assert_eq!(
            Candidate::new(ADDED, summary(2, 0, true), add()),
            Err(Error::Document)
        );
        let mut unversioned = summary(2, 8, true);
        unversioned.version = 0;
        assert_eq!(
            Candidate::new(ADDED, unversioned, add()),
            Err(Error::Document)
        );
        // Bytes that are not a task document never reach the intent.
        assert_eq!(
            Candidate::new(
                b"7\tdone\tFirst\n",
                summary(1, 7, true),
                Edit::Done { id: 7 }
            ),
            Err(Error::Document)
        );
        let mut oversized = [b'x'; MAX_BYTES + 1];
        oversized[..16].copy_from_slice(b"rustic-tasks-v1\n");
        assert_eq!(
            Candidate::new(&oversized, summary(0, 7, true), Edit::Done { id: 7 }),
            Err(Error::Document)
        );
    }

    #[test]
    fn chunks_assemble_only_the_declared_total() {
        let mut builder = Candidate::begin(ADDED.len(), summary(2, 8, true), add()).unwrap();
        for chunk in ADDED.chunks(32) {
            assert!(!builder.complete());
            builder.push(chunk).unwrap();
        }
        assert!(builder.complete());
        assert_eq!(builder.remaining(), 0);
        assert_eq!(
            builder.finish().unwrap(),
            Candidate::new(ADDED, summary(2, 8, true), add()).unwrap()
        );
        // Overlong, empty and oversized streams are refused as they arrive.
        let mut builder = Candidate::begin(ADDED.len(), summary(2, 8, true), add()).unwrap();
        builder.push(&ADDED[..32]).unwrap();
        assert_eq!(builder.remaining(), ADDED.len() - 32);
        assert_eq!(builder.push(ADDED), Err(Error::Document));
        assert_eq!(builder.push(&[]), Err(Error::Document));
        assert_eq!(
            Candidate::begin(MAX_BYTES + 1, summary(2, 8, true), add()).err(),
            Some(Error::Document)
        );
        assert_eq!(
            Candidate::begin(0, summary(2, 8, true), add()).err(),
            Some(Error::Document)
        );
    }

    #[test]
    fn an_incomplete_or_mismatched_stream_never_becomes_a_candidate() {
        let mut builder = Candidate::begin(ADDED.len(), summary(2, 8, true), add()).unwrap();
        builder.push(&ADDED[..32]).unwrap();
        assert!(!builder.complete());
        assert_eq!(builder.finish(), Err(Error::Document));
        // A complete stream still faces the document checks.
        let mut builder = Candidate::begin(DONE.len(), summary(1, 7, true), add()).unwrap();
        builder.push(DONE).unwrap();
        assert!(builder.complete());
        assert_eq!(builder.finish(), Err(Error::Document));
    }
}
