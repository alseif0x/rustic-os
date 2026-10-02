// SPDX-License-Identifier: Apache-2.0
//! Coherent queued activity and retained-record observation projections.
use super::super::scope::Scope7;
use super::{Candidate7, Scheduler7, Ticket7};
use rustic_abi::files::admission as a;

impl Scheduler7 {
    pub(super) fn observe(
        &self,
        candidate: Candidate7,
        ticket_index: Option<usize>,
    ) -> a::ObservationV2 {
        if let Some(index) = ticket_index {
            let ticket = self.tickets[index].unwrap();
            return a::ObservationV2::Active(queued(candidate.scope, ticket));
        }
        a::ObservationV2::Retained {
            status: candidate.scope.status,
            prevention: candidate
                .record
                .prevention
                .map(crate::admission::prevention_reason),
        }
    }
}

pub(super) fn activity(
    candidate: Scope7,
    ticket: Ticket7,
    op: u8,
    context: u32,
) -> Result<rustic_abi::files::Packet, rustic_abi::files::Error> {
    queued(candidate, ticket).packet(op, context)
}

fn queued(candidate: Scope7, ticket: Ticket7) -> a::Activity {
    a::Activity {
        id: candidate.id,
        service_instance: candidate.instance,
        phase: a::ActivityPhase::Queued,
        cancel_requested: ticket.stop,
        io_pending: false,
    }
}

pub(super) fn reply(
    view: a::ObservationV2,
    packet: rustic_abi::files::Packet,
) -> Result<rustic_abi::files::Packet, rustic_abi::files::Error> {
    crate::admission::observation_reply(view, packet)
}
