// SPDX-License-Identifier: Apache-2.0
//! A read-only view after the dispatcher's fresh subject/scope/authority checks.
use super::{Candidate, ExecutionQueue};
use crate::ActiveExecution;
use rustic_abi::files::{Error, admission as a};
use rustic_fs::AdmissionState;

impl ExecutionQueue {
    pub(super) fn observe(
        &self,
        candidate: &Candidate,
        active: Option<&ActiveExecution>,
    ) -> Result<a::ObservationV2, Error> {
        let scope = candidate.scope;
        if let Some(index) = self
            .tickets
            .iter()
            .position(|t| t.is_some_and(|t| t.id == scope.id))
        {
            let ticket = self.tickets[index].unwrap();
            let view = if index == 0 && self.running {
                let mut view = active.ok_or(Error::Busy)?.observation();
                if view.id != scope.id {
                    return Err(Error::Protocol);
                }
                view.cancel_requested |= ticket.stop;
                if view.cancel_requested && view.phase == a::ActivityPhase::Running {
                    view.phase = a::ActivityPhase::Stopping;
                }
                view
            } else {
                a::Activity {
                    id: scope.id,
                    service_instance: scope.instance,
                    phase: a::ActivityPhase::Queued,
                    cancel_requested: ticket.stop,
                    io_pending: false,
                }
            };
            return Ok(a::ObservationV2::Active(view));
        }
        Ok(a::ObservationV2::Retained {
            status: a::Status {
                id: scope.id,
                service_instance: scope.instance,
                state: match candidate.status.state {
                    AdmissionState::Admitted => a::State::Admitted,
                    AdmissionState::Cancelled => a::State::Cancelled,
                    AdmissionState::Committed => a::State::Committed,
                },
                terminal: candidate.status.terminal,
            },
            prevention: candidate
                .status
                .prevention
                .map(super::super::observation::reason),
        })
    }
}
