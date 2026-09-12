// SPDX-License-Identifier: Apache-2.0
//! Compact diagnostic output from the same typed SDK used by the manual client.
use rustic_sdk::files::{
    Client, Error,
    admission::AdmissionId,
    lifecycle::{Failure, State},
};

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
    let (state, detail, failure) = match operation.state {
        State::Prepared => (1, 0, 0),
        State::Queued { stop_pending } => (2, stop_pending as u64, 0),
        State::Running { stop_pending } => (3, stop_pending as u64, 0),
        State::Reconciling { stop_pending } => (4, stop_pending as u64, 0),
        State::Succeeded { completion_id } => (5, completion_id.sequence(), 0),
        State::Cancelled => (6, 0, 0),
        State::Failed { failure } => (
            7,
            0,
            match failure {
                Failure::VersionConflict => 1,
                Failure::AccessDenied => 2,
            },
        ),
        State::Prevented => (8, 0, 0),
    };
    Ok([0, state, detail, 0, failure, 0, 0, 0])
}
