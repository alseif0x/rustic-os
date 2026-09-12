// SPDX-License-Identifier: Apache-2.0
//! Diagnostic rendering shared by explicit-ID and client-owned lifecycle calls.
use rustic_sdk::files::lifecycle::{Failure, Operation, State};

pub(crate) fn report(operation: Operation) -> [u64; 8] {
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
    [0, state, detail, 0, failure, 0, 0, 0]
}
