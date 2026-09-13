// SPDX-License-Identifier: Apache-2.0
//! Native tasks frontend: syntax, transport and owner-client persistence compose here.
#[cfg(feature = "tasks-acceptance")]
mod acceptance;
mod journal;
mod mutation;
mod transport;
use super::{Error, Session, argument, exact, output};
#[cfg(feature = "tasks-acceptance")]
pub(super) use acceptance::execute as write_acceptance;
use rustic_sdk::{abi::supervisor as s, files};
use rustic_shell::parser::Args;
use rustic_tasks_contract::preview::Edit;

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
            mutation::recover(session)
        }
        "forget" => {
            exact(args, 3)?;
            let key = argument(args, 2)?
                .parse::<u64>()
                .map_err(|_| Error::Usage)?;
            let (saved, _) = journal::load(session)?.ok_or(Error::TaskJournal)?;
            if saved.version != key {
                return Err(Error::TaskPending(saved.version));
            }
            journal::clear(session, saved)?;
            output::text(
                "Task recovery record forgotten. This does not cancel or undo a submitted effect.\r\n",
            );
            Ok(())
        }
        "list" => {
            exact(args, 3)?;
            let scope = session.files.resolve(session.cwd, argument(args, 2)?)?;
            transport::run(session, scope, None, false).map(|_| ())
        }
        "preview" => {
            exact(args, 5)?;
            let edit = parse_edit(argument(args, 2)?, argument(args, 4)?)?;
            let scope = session.files.resolve(session.cwd, argument(args, 3)?)?;
            transport::run(session, scope, Some(edit), false).map(|_| ())
        }
        action @ ("add" | "done") => {
            exact(args, 4)?;
            let edit = parse_edit(action, argument(args, 3)?)?;
            mutation::apply(session, argument(args, 2)?, edit)
        }
        _ => Err(Error::Usage),
    }
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
