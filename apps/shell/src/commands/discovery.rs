// SPDX-License-Identifier: Apache-2.0
//! Show what the bound file service implements. Availability is not permission.
use super::*;
use rustic_sdk::abi::services::Method;

pub(super) fn execute(session: &mut Session, args: &Args<'_>) -> Result<(), Error> {
    exact(args, 1)?;
    let report = session.files.capabilities()?;
    for method in Method::ALL {
        output::format(format_args!(
            "capability-v1 method={} availability={}\r\n",
            method.name(),
            report.of(method).name()
        ));
    }
    let bounds = report.bounds;
    output::format(format_args!(
        "capability-bounds-v1 max_inline_bytes={} max_page_items={} receipt_capacity={}\r\n",
        bounds.max_inline_bytes, bounds.max_page_items, bounds.receipt_capacity
    ));
    Ok(())
}
