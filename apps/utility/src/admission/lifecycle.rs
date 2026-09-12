// SPDX-License-Identifier: Apache-2.0
//! Compact diagnostic output from the same typed SDK used by the manual client.
use rustic_sdk::files::{Client, Error, admission::AdmissionId};

pub(super) fn inspect(
    files: &mut Client,
    id: AdmissionId,
    profile: u64,
) -> Result<[u64; 8], Error> {
    let operation = if profile == rustic_sdk::abi::supervisor::actor::flags::SELECTED {
        files.inspect_selected(id)?
    } else if profile == rustic_sdk::abi::supervisor::actor::flags::NEGOTIATED {
        files
            .negotiate_lifecycle(rustic_sdk::abi::services::Method::OperationsGet)?
            .inspect(id)?
    } else {
        files.operation_inspect(id)?
    };
    Ok(crate::lifecycle::report(operation))
}
