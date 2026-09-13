// SPDX-License-Identifier: Apache-2.0
//! Validate task edit commands and retain one immutable replacement candidate.

use rustic_tasks_contract::{
    Document, Error as DocumentError, MAX_BYTES, MAX_TASKS, MAX_TITLE, State,
};

use super::render;

/// One application-owned task edit request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command<'a> {
    /// Append an open task with this printable ASCII title.
    Add(&'a [u8]),
    /// Mark the task with this nonzero stable ID done.
    Done(u32),
}

/// Errors found before a replacement is handed to the file service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The input does not satisfy the version-pinned task document contract.
    InvalidDocument,
    /// An add title is empty, too long or contains a non-printable byte.
    InvalidTitle,
    /// A done command used the reserved zero ID.
    InvalidId,
    /// A done command named no task in the document.
    TaskNotFound,
    /// The task or replacement byte bound would be exceeded.
    Capacity,
    /// No successor exists for the greatest existing task ID.
    IdExhausted,
}

/// A fixed-size, immutable replacement candidate and its task effect.
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Planned {
    bytes: [u8; MAX_BYTES],
    len: usize,
    task_id: u32,
    changed: bool,
}

impl Planned {
    /// Return exactly the canonical replacement bytes, without zero padding.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// Return the number of bytes in the replacement candidate.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return whether the replacement candidate has no bytes.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return the stable task ID affected by this plan.
    pub const fn task_id(&self) -> u32 {
        self.task_id
    }

    /// Return whether applying this plan changes the document bytes.
    pub const fn changed(&self) -> bool {
        self.changed
    }

    fn from_original(original: &[u8], task_id: u32) -> Self {
        let mut bytes = [0; MAX_BYTES];
        bytes[..original.len()].copy_from_slice(original);
        Self {
            bytes,
            len: original.len(),
            task_id,
            changed: false,
        }
    }

    fn rendered(
        document: &Document,
        appended: Option<(u32, &[u8])>,
        done_id: Option<u32>,
        task_id: u32,
    ) -> Result<Self, Error> {
        let mut bytes = [0; MAX_BYTES];
        let len =
            render::document(document, appended, done_id, &mut bytes).ok_or(Error::Capacity)?;
        Ok(Self {
            bytes,
            len,
            task_id,
            changed: true,
        })
    }
}

impl AsRef<[u8]> for Planned {
    fn as_ref(&self) -> &[u8] {
        self.bytes()
    }
}

/// Plan one bounded task edit without changing `original` or contacting a service.
pub fn plan(original: &[u8], command: Command<'_>) -> Result<Planned, Error> {
    let document = Document::parse(original).map_err(document_error)?;
    match command {
        Command::Add(title) => add(&document, title),
        Command::Done(id) => done(original, &document, id),
    }
}

fn add(document: &Document, title: &[u8]) -> Result<Planned, Error> {
    validate_title(title)?;
    if document.len() == MAX_TASKS {
        return Err(Error::Capacity);
    }

    let mut greatest = 0;
    for index in 0..document.len() {
        let task = document.get(index).expect("document length bounds get");
        greatest = greatest.max(task.id);
    }
    let task_id = greatest.checked_add(1).ok_or(Error::IdExhausted)?;
    Planned::rendered(document, Some((task_id, title)), None, task_id)
}

fn done(original: &[u8], document: &Document, id: u32) -> Result<Planned, Error> {
    if id == 0 {
        return Err(Error::InvalidId);
    }
    let mut task = None;
    for index in 0..document.len() {
        let candidate = document.get(index).expect("document length bounds get");
        if candidate.id == id {
            task = Some(*candidate);
            break;
        }
    }
    let task = task.ok_or(Error::TaskNotFound)?;
    if task.state == State::Done {
        return Ok(Planned::from_original(original, id));
    }
    Planned::rendered(document, None, Some(id), id)
}

fn validate_title(title: &[u8]) -> Result<(), Error> {
    if title.is_empty()
        || title.len() > MAX_TITLE
        || title.iter().any(|byte| !(0x20..=0x7e).contains(byte))
    {
        return Err(Error::InvalidTitle);
    }
    Ok(())
}

fn document_error(error: DocumentError) -> Error {
    match error {
        DocumentError::Invalid => Error::InvalidDocument,
        DocumentError::Capacity => Error::Capacity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &[u8] = b"rustic-tasks-v1\n";

    #[test]
    fn add_uses_one_and_serializes_exactly() {
        let original = EMPTY;
        let planned = plan(original, Command::Add(b"Review kernel")).unwrap();

        assert_eq!(
            planned.bytes(),
            b"rustic-tasks-v1\n1\topen\tReview kernel\n"
        );
        assert_eq!(planned.len(), planned.bytes().len());
        assert_eq!(planned.task_id(), 1);
        assert!(planned.changed());
        assert_eq!(original, EMPTY);
    }

    #[test]
    fn add_uses_the_greatest_id_without_reordering_existing_tasks() {
        let original = b"rustic-tasks-v1\n42\tdone\tBoot the OS\n7\topen\tReview kernel\n";
        let planned = plan(original, Command::Add(b"Measure boot")).unwrap();

        assert_eq!(
            planned.bytes(),
            b"rustic-tasks-v1\n42\tdone\tBoot the OS\n7\topen\tReview kernel\n43\topen\tMeasure boot\n"
        );
        assert_eq!(planned.task_id(), 43);
        assert!(planned.changed());
        assert_eq!(
            original,
            b"rustic-tasks-v1\n42\tdone\tBoot the OS\n7\topen\tReview kernel\n"
        );
    }

    #[test]
    fn done_changes_only_the_selected_open_task_and_preserves_order() {
        let original = b"rustic-tasks-v1\n7\topen\tReview kernel\n42\tdone\tBoot the OS\n";
        let planned = plan(original, Command::Done(7)).unwrap();

        assert_eq!(
            planned.bytes(),
            b"rustic-tasks-v1\n7\tdone\tReview kernel\n42\tdone\tBoot the OS\n"
        );
        assert_eq!(planned.task_id(), 7);
        assert!(planned.changed());
        assert_eq!(
            original,
            b"rustic-tasks-v1\n7\topen\tReview kernel\n42\tdone\tBoot the OS\n"
        );
    }

    #[test]
    fn invalid_documents_and_titles_are_rejected() {
        assert_eq!(
            plan(b"rustic-tasks-v2\n", Command::Add(b"x")),
            Err(Error::InvalidDocument)
        );
        for title in [b"".as_slice(), b"bad\n".as_slice(), &[0x1f], &[0x7f]] {
            assert_eq!(
                plan(EMPTY, Command::Add(title)),
                Err(Error::InvalidTitle),
                "title={title:?}"
            );
        }
        let long = [b'x'; MAX_TITLE + 1];
        assert_eq!(plan(EMPTY, Command::Add(&long)), Err(Error::InvalidTitle));
        assert_eq!(plan(EMPTY, Command::Done(0)), Err(Error::InvalidId));
        assert_eq!(plan(EMPTY, Command::Done(1)), Err(Error::TaskNotFound));
    }

    #[test]
    fn add_rejects_an_exhausted_greatest_id() {
        let original = b"rustic-tasks-v1\n4294967295\tdone\tFinished\n";
        assert_eq!(
            plan(original, Command::Add(b"another")),
            Err(Error::IdExhausted)
        );
    }

    #[test]
    fn add_rejects_a_full_document() {
        let original = b"rustic-tasks-v1\n1\topen\t1\n2\topen\t2\n3\topen\t3\n4\topen\t4\n5\topen\t5\n6\topen\t6\n7\topen\t7\n8\topen\t8\n9\topen\t9\n10\topen\t10\n11\topen\t11\n12\topen\t12\n13\topen\t13\n14\topen\t14\n15\topen\t15\n16\topen\t16\n";
        assert_eq!(
            plan(original, Command::Add(b"seventeenth")),
            Err(Error::Capacity)
        );
    }

    #[test]
    fn done_on_done_is_an_exact_noop_and_retains_original() {
        let original = b"rustic-tasks-v1\n9\tdone\tAlready complete\n";
        let planned = plan(original, Command::Done(9)).unwrap();

        assert_eq!(planned.bytes(), original);
        assert_eq!(planned.task_id(), 9);
        assert!(!planned.changed());
        assert_eq!(original, b"rustic-tasks-v1\n9\tdone\tAlready complete\n");
    }
}
