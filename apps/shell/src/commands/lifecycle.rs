// SPDX-License-Identifier: Apache-2.0
//! Manual presentation of the narrow, versioned logical lifecycle.
use super::*;
use rustic_sdk::files::lifecycle::{Disposition, Failure, State};

pub(super) fn execute(s: &mut Session, args: &Args<'_>) -> Result<(), Error> {
    exact(args, 2)?;
    let id = argument(args, 1)?.parse()?;
    if argument(args, 0)? == "request-operation-cancel" {
        let ack = s.files.operation_cancel(id)?;
        let disposition = match ack.disposition {
            Disposition::Requested => "requested",
            Disposition::AlreadyRequested => "already_requested",
            Disposition::TooLate => "too_late",
        };
        output::format(format_args!(
            "operation-cancel-v2 id={} disposition={}\r\n",
            ack.id, disposition
        ));
        return Ok(());
    }
    let operation = s.files.operation_inspect(id)?;
    let (state, effect) = match operation.state {
        State::Prepared => ("prepared", "none"),
        State::Queued { .. } => ("queued", "none"),
        State::Running { .. } => ("running", "none"),
        State::Reconciling { .. } => ("reconciling", "unknown"),
        State::Succeeded { .. } => ("succeeded", "committed"),
        State::Cancelled => ("cancelled", "none"),
        State::Failed { .. } => ("failed", "none"),
        State::Prevented => ("prevented", "none"),
    };
    output::format(format_args!(
        "operation-v2 id={} service_instance={} state={} effect={}",
        operation.id, operation.service_instance, state, effect
    ));
    match operation.state {
        State::Queued { stop_pending }
        | State::Running { stop_pending }
        | State::Reconciling { stop_pending } => {
            output::format(format_args!(" stop_pending={}", stop_pending as u8));
        }
        State::Succeeded { completion_id } => {
            output::format(format_args!(" completion_id={}", completion_id))
        }
        State::Failed { failure } => output::format(format_args!(
            " failure={}",
            match failure {
                Failure::VersionConflict => "version_conflict",
                Failure::AccessDenied => "access_denied",
            }
        )),
        _ => (),
    }
    output::text("\r\n");
    Ok(())
}
