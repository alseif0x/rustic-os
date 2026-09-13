// SPDX-License-Identifier: Apache-2.0
//! Deterministic owner client for the native tasks lifecycle acceptance.
//!
//! The fixture resolves the supplied object once and then drives the actual
//! supervisor RPC.  Each case owns one lifecycle boundary; no case reads or
//! parses the task document itself.

mod cancel;
mod concurrent;
mod expiry;
mod support;

use crate::commands::{Error, argument, exact};
use crate::session::Session;

pub(super) fn execute(
    session: &mut Session,
    args: &rustic_shell::parser::Args<'_>,
) -> Result<(), Error> {
    exact(args, 2)?;
    let scope = session.files.resolve(session.cwd, argument(args, 1)?)?;
    if scope == 0 {
        return Err(Error::Service(4));
    }
    cancel::run(session, scope)?;
    concurrent::run(session, scope)?;
    expiry::run(session, scope)?;
    Ok(())
}
