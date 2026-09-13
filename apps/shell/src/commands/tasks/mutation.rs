// SPDX-License-Identifier: Apache-2.0
//! Owner-client application of an immutable native plan; never implicit replay.
use super::{Error, Session, journal, output, transport};
use rustic_sdk::{
    abi::services::{Availability, Method},
    files::{
        self,
        operation::{Lookup, Operation},
        read,
    },
};
use rustic_shell::task_intent::Intent;
use rustic_tasks_contract::preview::Edit;

pub(super) fn apply(session: &mut Session, path: &str, edit: Edit) -> Result<(), Error> {
    apply_cut(session, path, edit, Cut::None)
}

pub(super) enum Cut {
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

pub(super) fn apply_cut(
    session: &mut Session,
    path: &str,
    edit: Edit,
    _cut: Cut,
) -> Result<(), Error> {
    journal::idle(session)?;
    if session.files.capabilities()?.of(Method::FilesReplace) != Availability::Available {
        return Err(Error::TaskEnable);
    }
    let scope = session.files.resolve(session.cwd, path)?;
    if journal::locate(session)?.is_some_and(|journal| journal.id == scope) {
        return Err(Error::TaskJournal);
    }
    let candidate = transport::run(session, scope, Some(edit), true)?.ok_or(Error::Service(4))?;
    let metadata = session.files.stat(scope)?;
    if metadata.version != candidate.summary.version {
        return Err(files::Error::Version.into());
    }
    let mut original = [0; 1024];
    let info = journal::snapshot(session, metadata, &mut original)?;
    if !candidate.summary.changed {
        if original[..info.length] != candidate.bytes[..candidate.length] {
            return Err(files::Error::Protocol.into());
        }
        output::format(format_args!(
            "task={} unchanged version={}\r\n",
            candidate.summary.task_id, info.version
        ));
        return Ok(());
    }
    let intent = Intent::new(
        info.references,
        info.version,
        info.retry_epoch,
        edit,
        candidate.summary.task_id,
        &candidate.bytes[..candidate.length],
    )?;
    #[cfg(not(feature = "tasks-acceptance"))]
    let lose_journal_reply = false;
    #[cfg(feature = "tasks-acceptance")]
    let lose_journal_reply = matches!(_cut, Cut::LostJournal);
    let saved = journal::retain(session, &intent, lose_journal_reply)?;
    output::format(format_args!(
        "task intent={} retained before submission\r\n",
        saved.version
    ));
    let request = intent.request(saved.version)?;
    #[cfg(feature = "tasks-acceptance")]
    match _cut {
        Cut::Prepared => return Err(files::Error::Uncertain.into()),
        Cut::HumanConflict => {
            session.files.replace(
                scope,
                request.expected_version.value(),
                b"human edit survives",
            )?;
        }
        Cut::LostReply => {
            return super::acceptance::discard_reply(session, request, intent.candidate());
        }
        Cut::None | Cut::LostJournal => {}
    }
    match session.files.replace_file(request, intent.candidate()) {
        Ok(operation) => finish(session, saved, &intent, operation, false),
        Err(error) => {
            // These canonical server refusals prove this synchronous attempt
            // did not publish. Transport/process-death ambiguity retains intent.
            if matches!(
                error,
                files::Error::Full
                    | files::Error::Version
                    | files::Error::Denied
                    | files::Error::Revoked
                    | files::Error::Expired
                    | files::Error::ReadOnly
            ) {
                journal::clear(session, saved)?;
            } else {
                output::text(
                    "Intent retained; use tasks recover (restart files first if disconnected). No automatic retry.\r\n",
                );
            }
            Err(error.into())
        }
    }
}

pub(super) fn recover(session: &mut Session) -> Result<(), Error> {
    let Some((saved, intent)) = journal::load(session)? else {
        output::text(
            "No retained task intent. This does not prove a forgotten effect never happened.\r\n",
        );
        return Ok(());
    };
    let request = intent.request(saved.version)?;
    // Fresh authority is supplied by the current owner binding. This performs
    // no target writes and never rebuilds a replacement from current contents.
    let operation = session
        .files
        .operation_get(Lookup::Retry {
            workspace: request.workspace,
            retry: request.retry,
        })
        .map_err(|error| {
            output::format(format_args!(
                "intent={} unresolved; target not resubmitted\r\n",
                saved.version
            ));
            Error::File(error)
        })?;
    finish(session, saved, &intent, operation, true)
}

fn finish(
    session: &mut Session,
    saved: files::Metadata,
    intent: &Intent,
    operation: Operation,
    recovered: bool,
) -> Result<(), Error> {
    if !intent.matches(saved.version, &operation) {
        return Err(files::Error::Uncertain.into());
    }
    let mut actual = [0; 1024];
    let info = session.files.read_range(read::Request { workspace: operation.workspace,
        resource: operation.resource, expected_version: Some(operation.version), offset: 0, length: 1024 }, &mut actual)
        .map_err(|error| {
            output::format(format_args!("intent={} committed at version={}; current bytes could not be verified; intent retained\r\n", saved.version, operation.version));
            Error::File(error)
        })?;
    if !info.eof
        || info.length != intent.candidate().len()
        || &actual[..info.length] != intent.candidate()
    {
        return Err(files::Error::Uncertain.into());
    }
    output::format(format_args!(
        "task={} {} version={} intent={} bytes verified\r\n",
        intent.task_id(),
        if recovered { "recovered" } else { "applied" },
        operation.version,
        saved.version
    ));
    if let Err(error) = journal::clear(session, saved) {
        output::text(
            "Effect verified; intent cleanup incomplete. Use tasks recover before another edit.\r\n",
        );
        return Err(error);
    }
    Ok(())
}
