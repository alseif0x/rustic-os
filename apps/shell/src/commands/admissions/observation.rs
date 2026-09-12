// SPDX-License-Identifier: Apache-2.0
//! Human-readable rendering of a single typed service observation.
use crate::output;
use rustic_sdk::files::admission::{
    ActivityPhase, OBSERVATION_V2, OBSERVATION_VERSION, Observation, ObservationV2,
    PreventionReason, State,
};

pub(super) fn print(view: Observation) {
    render(view, OBSERVATION_VERSION, None);
}
pub(super) fn print_v2(view: ObservationV2) {
    let cause = match view {
        ObservationV2::Retained { prevention, .. } => Some(match prevention {
            None => "none",
            Some(PreventionReason::Unknown) => "unknown",
            Some(PreventionReason::Requested) => "requested",
            Some(PreventionReason::VersionConflict) => "version_conflict",
            Some(PreventionReason::AuthorityLost) => "authority_lost",
        }),
        ObservationV2::Active(_) => None,
    };
    render(view.coarse(), OBSERVATION_V2, cause);
}
fn render(view: Observation, profile: u32, cause: Option<&str>) {
    output::format(format_args!(
        "admission-observation-v{} profile={} id={} service_instance={} ",
        profile,
        profile,
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
                "kind=retained state={} terminal={}",
                state, v.terminal
            ));
            if let Some(cause) = cause {
                output::format(format_args!(" prevention={}", cause));
            }
            output::text("\r\n");
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
