// SPDX-License-Identifier: Apache-2.0
//! Owner-client application of an immutable native plan; never implicit replay.
use crate::{
    Applied, Authority, Candidate, Committed, Error, Note, Record, Recovery, Relay, Report,
    intent::Intent, journal, transport,
};
use rustic_sdk::{
    abi::services::{Availability, Method},
    files::{
        self, Metadata,
        operation::{Lookup, Operation},
        read,
    },
};
use rustic_tasks_contract::preview::Edit;

/// An explicit failure cut. Ordinary builds only have `None`.
///
/// Every cut is taken inside the single submission path, after the plan exists
/// and before, during or after the one exchange that publishes it, so all four
/// are available to a caller that only holds a plan: none of them needs the
/// planning relay.
///
/// - `Prepared` retains the intent and stops before submitting it.
/// - `HumanConflict` writes foreign bytes over the target first, so the one
///   submission is refused by version. The foreign bytes stay on the target.
/// - `LostReply` submits the replacement and discards its reply.
/// - `LostJournal` retains the intent but loses the acknowledgement of the
///   retention, so the submission never happens.
pub enum Cut {
    None,
    #[cfg(feature = "tasks-acceptance")]
    Prepared,
    #[cfg(feature = "tasks-acceptance")]
    HumanConflict,
    #[cfg(feature = "tasks-acceptance")]
    LostReply,
    #[cfg(feature = "tasks-acceptance")]
    LostJournal,
}

pub(crate) fn apply_cut<L: Relay, R: Report>(
    relay: &mut L,
    report: &mut R,
    record: &Record<'_>,
    cwd: u32,
    path: &str,
    edit: Edit,
    cut: Cut,
) -> Result<Applied, Error> {
    guard(relay, record)?;
    let scope = relay.files().resolve(cwd, path)?;
    distinct(relay, record, scope)?;
    // One planning path: applying an edit plans it exactly as a caller that only
    // wants the plan does.
    let candidate = transport::plan(relay, scope, edit)?;
    commit(relay, report, record, scope, &candidate, cut)
}

/// Applies a plan the caller already holds, with an explicit failure cut. The
/// same preconditions and the same single submission apply: only the planning
/// exchange is the caller's.
///
/// The cut is honoured by the one shared submission path, so a candidate applied
/// by a client that cannot plan fails exactly where a planned edit does.
pub(crate) fn apply_candidate_cut<A: Authority, R: Report>(
    authority: &mut A,
    report: &mut R,
    record: &Record<'_>,
    target: u32,
    candidate: &Candidate,
    cut: Cut,
) -> Result<Applied, Error> {
    guard(authority, record)?;
    distinct(authority, record, target)?;
    commit(authority, report, record, target, candidate, cut)
}

/// The record must be idle and replacement must be enabled before a plan is even
/// requested, so a blocked intent is reported before any navigation error.
fn guard<A: Authority>(authority: &mut A, record: &Record<'_>) -> Result<(), Error> {
    journal::idle(authority, record)?;
    if authority.files().capabilities()?.of(Method::FilesReplace) != Availability::Available {
        return Err(Error::Enable);
    }
    Ok(())
}

/// The journal object may not be the target of the edit it records.
fn distinct<A: Authority>(
    authority: &mut A,
    record: &Record<'_>,
    target: u32,
) -> Result<(), Error> {
    if journal::locate(authority, record)?.is_some_and(|journal| journal.id == target) {
        return Err(Error::Journal);
    }
    Ok(())
}

fn commit<A: Authority, R: Report>(
    authority: &mut A,
    report: &mut R,
    record: &Record<'_>,
    scope: u32,
    candidate: &Candidate,
    _cut: Cut,
) -> Result<Applied, Error> {
    let summary = candidate.summary();
    let metadata = authority.files().stat(scope)?;
    if metadata.version != summary.version {
        return Err(files::Error::Version.into());
    }
    let mut original = [0; 1024];
    let info = journal::snapshot(authority, metadata, &mut original)?;
    if !summary.changed {
        if original[..info.length] != *candidate.bytes() {
            return Err(files::Error::Protocol.into());
        }
        return Ok(Applied::Unchanged {
            task_id: summary.task_id,
            version: info.version,
        });
    }
    let intent = Intent::new(
        info.references,
        info.version,
        info.retry_epoch,
        candidate.edit(),
        summary.task_id,
        candidate.bytes(),
    )?;
    #[cfg(not(feature = "tasks-acceptance"))]
    let lose_journal_reply = false;
    #[cfg(feature = "tasks-acceptance")]
    let lose_journal_reply = matches!(_cut, Cut::LostJournal);
    let saved = journal::retain(authority, report, record, &intent, lose_journal_reply)?;
    report.note(Note::Retained {
        journal: saved.version,
    });
    let request = intent.request(saved.version)?;
    #[cfg(feature = "tasks-acceptance")]
    match _cut {
        Cut::Prepared => return Err(files::Error::Uncertain.into()),
        Cut::HumanConflict => {
            authority.files().replace(
                scope,
                request.expected_version.value(),
                b"human edit survives",
            )?;
        }
        Cut::LostReply => {
            crate::acceptance::discard_reply(authority, report, request, intent.candidate())?;
            return Err(files::Error::Uncertain.into());
        }
        Cut::None | Cut::LostJournal => {}
    }
    match authority.files().replace_file(request, intent.candidate()) {
        Ok(operation) => {
            finish(authority, report, saved, &intent, operation).map(Applied::Committed)
        }
        Err(error) => {
            let refusal = Error::File(error);
            // These canonical server refusals prove this synchronous attempt
            // did not publish. Transport/process-death ambiguity retains intent.
            if refusal.conclusive() {
                journal::clear(authority, saved)?;
            } else {
                report.note(Note::Ambiguous);
            }
            Err(refusal)
        }
    }
}

pub(crate) fn recover<A: Authority, R: Report>(
    authority: &mut A,
    report: &mut R,
    record: &Record<'_>,
) -> Result<Recovery, Error> {
    let Some((saved, intent)) = journal::load(authority, record)? else {
        return Ok(Recovery::Absent);
    };
    let request = intent.request(saved.version)?;
    // Fresh authority is supplied by the current owner binding. This performs
    // no target writes and never rebuilds a replacement from current contents.
    let operation = authority
        .files()
        .operation_get(Lookup::Retry {
            workspace: request.workspace,
            retry: request.retry,
        })
        .map_err(|error| {
            report.note(Note::Unresolved {
                journal: saved.version,
            });
            Error::File(error)
        })?;
    finish(authority, report, saved, &intent, operation).map(Recovery::Recovered)
}

fn finish<A: Authority, R: Report>(
    authority: &mut A,
    report: &mut R,
    saved: Metadata,
    intent: &Intent,
    operation: Operation,
) -> Result<Committed, Error> {
    if !intent.matches(saved.version, &operation) {
        return Err(files::Error::Uncertain.into());
    }
    let mut actual = [0; 1024];
    let info = authority
        .files()
        .read_range(
            read::Request {
                workspace: operation.workspace,
                resource: operation.resource,
                expected_version: Some(operation.version),
                offset: 0,
                length: 1024,
            },
            &mut actual,
        )
        .map_err(|error| {
            report.note(Note::Unverified {
                journal: saved.version,
                version: operation.version,
            });
            Error::File(error)
        })?;
    if !info.eof
        || info.length != intent.candidate().len()
        || &actual[..info.length] != intent.candidate()
    {
        return Err(files::Error::Uncertain.into());
    }
    let committed = Committed {
        task_id: intent.task_id(),
        version: operation.version,
        journal: saved.version,
    };
    if let Err(error) = journal::clear(authority, saved) {
        report.note(Note::CleanupIncomplete(committed));
        return Err(error);
    }
    Ok(committed)
}
