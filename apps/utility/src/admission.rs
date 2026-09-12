// SPDX-License-Identifier: Apache-2.0
//! Owner-stepped native client; file service independently enforces each action.
mod lifecycle;
mod observation;
use rustic_sdk::{
    abi::{
        files::{Error, admission as a},
        supervisor::actor,
    },
    files::Client,
};
pub fn run(files: &mut Client, w: [u64; 8]) -> [u64; 8] {
    let result = (|| {
        let mut lineage = [0; 16];
        lineage[..8].copy_from_slice(&w[1].to_le_bytes());
        lineage[8..].copy_from_slice(&w[2].to_le_bytes());
        let id = a::AdmissionId::new(lineage, w[3])?;
        if w[5] == actor::flags::DISCARD_REPLY {
            // Readiness is not a decoded result; the owner reconciles separately.
            return super::live::discard_admission_reply(files, id, w[4] as u8).map(|()| [0; 8]);
        }
        if w[4] == a::OBSERVE as u64 {
            if matches!(
                w[5],
                actor::flags::LIFECYCLE | actor::flags::NEGOTIATED | actor::flags::SELECTED
            ) {
                return lifecycle::inspect(files, id, w[5]);
            }
            return observation::run(files, id, w[5] == actor::flags::OBSERVE_V2);
        }
        if w[4] == rustic_sdk::files::lifecycle::CANCEL as u64 {
            let ack = if w[5] == actor::flags::SELECTED {
                files.cancel_selected(id)?
            } else if w[5] == actor::flags::NEGOTIATED {
                files
                    .negotiate_lifecycle(rustic_sdk::abi::services::Method::OperationsCancel)?
                    .cancel(id)?
            } else {
                files.operation_cancel(id)?
            };
            return Ok([0, ack.disposition as u64, 0, 0, 0, 0, 0, 0]);
        }
        if w[4] == a::EXECUTE as u64 || w[4] == a::GET as u64 {
            let result = if w[4] == a::GET as u64 {
                files.admission_get(id)?
            } else {
                files.admission_execute(id)?
            };
            return Ok([
                0,
                match result.state {
                    a::State::Admitted => 1,
                    a::State::Cancelled => 2,
                    a::State::Committed => 3,
                },
                result.terminal,
                0,
                0,
                0,
                0,
                0,
            ]);
        }
        let result = if w[4] == a::SCHEDULE as u64 {
            files.admission_schedule(id)?
        } else if w[4] == a::ACTIVITY as u64 {
            files.admission_activity(id)?
        } else if w[4] == a::REQUEST_CANCEL as u64 {
            files.admission_request_cancel(id)?
        } else {
            return Err(Error::Invalid);
        };
        Ok([
            0,
            match result.phase {
                a::ActivityPhase::Queued => 4,
                a::ActivityPhase::Running => 1,
                a::ActivityPhase::Stopping => 2,
                a::ActivityPhase::Settling => 3,
            },
            result.cancel_requested as u64,
            result.io_pending as u64,
            0,
            0,
            0,
            0,
        ])
    })();
    result.unwrap_or_else(|error| [error as u64, 0, 0, 0, 0, 0, 0, 0])
}
