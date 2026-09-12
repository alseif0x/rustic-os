// SPDX-License-Identifier: Apache-2.0
//! Owner-stepped native client; file service independently enforces each action.
use rustic_sdk::{
    abi::files::{Error, admission as a},
    files::Client,
};
pub fn run(files: &mut Client, w: [u64; 8]) -> [u64; 8] {
    let result = (|| {
        let mut lineage = [0; 16];
        lineage[..8].copy_from_slice(&w[1].to_le_bytes());
        lineage[8..].copy_from_slice(&w[2].to_le_bytes());
        let id = a::AdmissionId::new(lineage, w[3])?;
        if w[4] == a::EXECUTE as u64 {
            let result = files.admission_execute(id)?;
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
        let result = if w[4] == a::ACTIVITY as u64 {
            files.admission_activity(id)?
        } else if w[4] == a::REQUEST_CANCEL as u64 {
            files.admission_request_cancel(id)?
        } else {
            return Err(Error::Invalid);
        };
        Ok([
            0,
            match result.phase {
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
