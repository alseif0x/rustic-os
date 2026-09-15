// SPDX-License-Identifier: Apache-2.0
//! Native tasks frontend: syntax and presentation over the shared owner client.
#[cfg(feature = "tasks-acceptance")]
mod acceptance;
mod hand;
mod render;
use super::{Error, Session, argument, exact, output};
#[cfg(feature = "tasks-acceptance")]
pub(super) use acceptance::execute as write_acceptance;
use render::Console;
use rustic_sdk::{abi::supervisor as s, files};
use rustic_shell::parser::Args;
use rustic_tasks_client::{Client, Cut, Record};
use rustic_tasks_contract::preview::Edit;

/// Where the shell keeps its own recovery record. A second semantic client keeps
/// its own, so neither client can mistake the other's unresolved intent for its
/// own; this constant is the single place the location is written.
const RECORD: Record<'static> = Record::path("/config", "tasks-intent");
const CLIENT: Client<'static> = Client::new(RECORD);

/// The object the shell's record occupies, when it already exists.
///
/// The record is never created to answer this question: a shell that has not
/// retained an intent yet simply has no record object, and the caller treats
/// that as nothing to collide with.
pub(super) fn record_object(session: &mut Session) -> Result<Option<u32>, Error> {
    let Record::Path { directory, name } = RECORD else {
        return Ok(None);
    };
    let directory = session.files.resolve(0, directory)?;
    match session.files.lookup(directory, name) {
        Ok(metadata) if !metadata.directory => Ok(Some(metadata.id)),
        Ok(_) | Err(files::Error::NotFound) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn execute(session: &mut Session, args: &Args<'_>) -> Result<(), Error> {
    match argument(args, 1)? {
        "enable" => {
            exact(args, 2)?;
            let reply = session.service([s::ENABLE_OPERATIONS, 0, 0, 0, 0, 0, 0, 0])?;
            files::Error::parse(reply[1] as u8)?;
            output::text("Task writes enabled; persistent storage format is at least v3.\r\n");
            Ok(())
        }
        "recover" => {
            exact(args, 2)?;
            render::recovery(CLIENT.recover(session, &mut Console::recovered())?);
            Ok(())
        }
        "forget" => {
            exact(args, 3)?;
            let key = argument(args, 2)?
                .parse::<u64>()
                .map_err(|_| Error::Usage)?;
            CLIENT.forget(session, key)?;
            output::text(
                "Task recovery record forgotten. This does not cancel or undo a submitted effect.\r\n",
            );
            Ok(())
        }
        "list" => {
            exact(args, 3)?;
            let scope = session.files.resolve(session.cwd, argument(args, 2)?)?;
            render::listing(&CLIENT.list(session, scope)?);
            Ok(())
        }
        "preview" => {
            exact(args, 5)?;
            let edit = parse_edit(argument(args, 2)?, argument(args, 4)?)?;
            let scope = session.files.resolve(session.cwd, argument(args, 3)?)?;
            render::listing(&CLIENT.preview(session, scope, edit)?);
            Ok(())
        }
        "hand" => hand::execute(session, args),
        action @ ("add" | "done") => {
            exact(args, 4)?;
            let edit = parse_edit(action, argument(args, 3)?)?;
            apply(session, argument(args, 2)?, edit, Cut::None)
        }
        _ => Err(Error::Usage),
    }
}

fn apply(session: &mut Session, path: &str, edit: Edit, cut: Cut) -> Result<(), Error> {
    let cwd = session.cwd;
    let applied = CLIENT.apply_cut(session, &mut Console::applied(), cwd, path, edit, cut)?;
    render::applied(applied);
    Ok(())
}

fn parse_edit(action: &str, value: &str) -> Result<Edit, Error> {
    match action {
        "add" => Edit::add(value.as_bytes()).ok_or(Error::Usage),
        "done" => {
            let id = value.parse::<u32>().map_err(|_| Error::Usage)?;
            if id == 0 || value.starts_with('0') || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Error::Usage);
            }
            Ok(Edit::Done { id })
        }
        _ => Err(Error::Usage),
    }
}
