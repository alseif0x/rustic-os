// SPDX-License-Identifier: Apache-2.0
//! Versioned profile, descriptor digest and current IPC responder presentation.
use super::*;
use rustic_sdk::abi::{files::negotiation as n, services::Method};

pub(super) fn execute(session: &mut Session, args: &Args<'_>) -> Result<(), Error> {
    exact(args, 2)?;
    let method = match argument(args, 1)? {
        "operations.get" => Method::OperationsGet,
        "operations.cancel" => Method::OperationsCancel,
        _ => return Err(Error::Usage),
    };
    if argument(args, 0)? == "select-lifecycle" {
        let d = session.files.select_lifecycle(method)?;
        output::format(format_args!(
            "lifecycle-selected method={} availability={}\r\n",
            method.name(),
            d.availability.name()
        ));
        return Ok(());
    }
    let selected = session.files.negotiate_lifecycle(method)?;
    let d = selected.descriptor();
    output::format(format_args!(
        "lifecycle-profile method={} version={} profile={} availability={} responder={} context={} retained={} tickets={} active={} sha256=",
        method.name(),
        n::VERSION,
        n::PROFILE,
        d.availability.name(),
        selected.responder(),
        selected.context(),
        d.limits.retained_operations,
        d.limits.execution_tickets,
        d.limits.active_publications,
    ));
    for byte in d.contract_sha256 {
        output::format(format_args!("{byte:02x}"));
    }
    output::text("\r\n");
    Ok(())
}
