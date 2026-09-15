// SPDX-License-Identifier: Apache-2.0
//! Explicit native failure cut selection; absent from ordinary builds.
use super::{Error, Session, apply, argument, exact, parse_edit};
use rustic_tasks_client::Cut;

pub(in crate::commands) fn execute(
    session: &mut Session,
    args: &rustic_shell::parser::Args<'_>,
) -> Result<(), Error> {
    exact(args, 5)?;
    let cut = match argument(args, 1)? {
        "prepared" => Cut::Prepared,
        "conflict" => Cut::HumanConflict,
        "lost-reply" => Cut::LostReply,
        "lost-journal" => Cut::LostJournal,
        _ => return Err(Error::Usage),
    };
    let edit = parse_edit(argument(args, 2)?, argument(args, 4)?)?;
    apply(session, argument(args, 3)?, edit, cut)
}
