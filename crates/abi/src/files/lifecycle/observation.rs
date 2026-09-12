// SPDX-License-Identifier: Apache-2.0
use crate::files::{
    Error,
    admission::{ActivityPhase, AdmissionId, ObservationV2, PreventionReason, State as Retained},
    operation::{Instance, OperationId},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    VersionConflict,
    AccessDenied,
}

/// Terminal variants deliberately have no historical stop flag. Storage does
/// not retain that history; even a completion may have raced an accepted stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Prepared,
    Queued { stop_pending: bool },
    Running { stop_pending: bool },
    Reconciling { stop_pending: bool },
    Succeeded { completion_id: OperationId },
    Cancelled,
    Failed { failure: Failure },
    Prevented,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operation {
    pub id: AdmissionId,
    pub service_instance: Instance,
    pub state: State,
}

impl TryFrom<ObservationV2> for Operation {
    type Error = Error;
    fn try_from(view: ObservationV2) -> Result<Self, Error> {
        // This public conversion must also reject manually constructed invalid
        // observations, not only rely on callers having decoded a wire reply.
        view.packet(0)?;
        let state = match view {
            ObservationV2::Active(v) => {
                let stop_pending = v.cancel_requested;
                match v.phase {
                    ActivityPhase::Queued => State::Queued { stop_pending },
                    ActivityPhase::Running | ActivityPhase::Stopping => {
                        State::Running { stop_pending }
                    }
                    ActivityPhase::Settling => State::Reconciling { stop_pending },
                }
            }
            ObservationV2::Retained { status, prevention } => match status.state {
                Retained::Admitted => State::Prepared,
                Retained::Committed => State::Succeeded {
                    completion_id: status.completion().ok_or(Error::Protocol)?,
                },
                Retained::Cancelled => match prevention.ok_or(Error::Protocol)? {
                    PreventionReason::Requested => State::Cancelled,
                    PreventionReason::Unknown => State::Prevented,
                    PreventionReason::VersionConflict => State::Failed {
                        failure: Failure::VersionConflict,
                    },
                    PreventionReason::AuthorityLost => State::Failed {
                        failure: Failure::AccessDenied,
                    },
                },
            },
        };
        Ok(Self {
            id: view.coarse().id(),
            service_instance: view.coarse().service_instance(),
            state,
        })
    }
}
