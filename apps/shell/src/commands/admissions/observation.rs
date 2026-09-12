// SPDX-License-Identifier: Apache-2.0
//! Human-readable rendering of a single typed service observation.
use crate::output;
use rustic_sdk::files::admission::{ActivityPhase, OBSERVATION_VERSION, Observation, State};

pub(super) fn print(view: Observation) {
    output::format(format_args!(
        "admission-observation-v1 profile={} id={} service_instance={} ",
        OBSERVATION_VERSION,
        view.id(),
        view.service_instance()
    ));
    match view {
        Observation::Retained(v) => {
            let state = match v.state {
                State::Admitted => "admitted",
                State::Cancelled => "cancelled",
                State::Committed => "committed",
            };
            output::format(format_args!(
                "kind=retained state={} terminal={}\r\n",
                state, v.terminal
            ));
            if let Some(completion) = v.completion() {
                output::format(format_args!("completion={}\r\n", completion));
            }
        }
        Observation::Active(v) => {
            let phase = match v.phase {
                ActivityPhase::Queued => "queued",
                ActivityPhase::Running => "running",
                ActivityPhase::Stopping => "stopping",
                ActivityPhase::Settling => "settling",
            };
            output::format(format_args!(
                "kind=active phase={} cancel_requested={} io_pending={}\r\n",
                phase, v.cancel_requested as u8, v.io_pending as u8
            ));
        }
    }
}
