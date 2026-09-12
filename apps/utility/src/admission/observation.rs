// SPDX-License-Identifier: Apache-2.0
//! Compact deterministic report of the same typed observation as the manual SDK.
use rustic_sdk::{
    abi::files::{Error, admission as a},
    files::Client,
};

pub(super) fn run(files: &mut Client, id: a::AdmissionId, v2: bool) -> Result<[u64; 8], Error> {
    let (view, reason) = if v2 {
        let detailed = files.admission_observe_v2(id)?;
        let reason = match detailed {
            a::ObservationV2::Retained { prevention, .. } => prevention.map_or(0, |r| r as u64),
            a::ObservationV2::Active(_) => 0,
        };
        (detailed.coarse(), reason)
    } else {
        (files.admission_observe(id)?, 0)
    };
    let values = match view {
        a::Observation::Retained(v) => [
            match v.state {
                a::State::Admitted => 1,
                a::State::Cancelled => 2,
                a::State::Committed => 3,
            },
            v.terminal,
            0,
        ],
        a::Observation::Active(v) => [
            0x10 | match v.phase {
                a::ActivityPhase::Running => 1,
                a::ActivityPhase::Stopping => 2,
                a::ActivityPhase::Settling => 3,
                a::ActivityPhase::Queued => 4,
            },
            v.cancel_requested as u64,
            v.io_pending as u64,
        ],
    };
    // The existing diagnostic report's `version` field carries the cause code
    // only for this explicitly selected profile. It is not a file version.
    Ok([0, values[0], values[1], values[2], reason, 0, 0, 0])
}
