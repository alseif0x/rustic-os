// SPDX-License-Identifier: Apache-2.0
//! Apply live authority to a scheduled or active V7 admission request.
use super::{Caller7, Scheduler7, Ticket7, TicketKind7};
use crate::v7::{admission::active::Active7, grants::Grants};
use rustic_abi::files::{
    CANCEL_RIGHT, Error, INSPECT_RIGHT, Packet, WRITE_RIGHT,
    admission::{self as a, AdmissionId},
    lifecycle::{self, CancelAck, Disposition},
};
use rustic_fs::format7::RecordState;

impl Scheduler7 {
    pub(in crate::v7) fn request(
        &mut self,
        grants: &Grants,
        active: Option<&mut Active7>,
        slot: usize,
        peer: u64,
        packet: Packet,
        now: u64,
    ) -> Packet {
        let result = (|| {
            crate::validation::request(&packet)?;
            if !super::is_lifecycle(packet.op) {
                return Err(Error::Busy);
            }
            let caller = Caller7 {
                slot,
                peer,
                context: packet.context,
            };
            let right = match packet.op {
                a::SCHEDULE => INSPECT_RIGHT | WRITE_RIGHT,
                a::REQUEST_CANCEL | lifecycle::CANCEL => CANCEL_RIGHT,
                _ => INSPECT_RIGHT,
            };
            // Establish current endpoint authority before looking up an
            // admission ID, so a missing or hidden identity never masks a
            // peer, context, expiry or rights denial.
            let live = grants.check(slot, peer, packet.context, now)?;
            live.holds(right)?;
            let id = if packet.op == lifecycle::CANCEL {
                CancelAck::decode_request(&packet)?
            } else {
                AdmissionId::decode(&packet)?
            };

            // The active execution owns the freshest coherent live view while
            // publication holds the volume. Its saved scope also prevents an
            // old retained admitted snapshot from hiding cancellation progress.
            let active_scope = active
                .as_deref()
                .filter(|current| current.scope.id == id)
                .map(|current| current.scope);
            let candidate = self.candidate(id);
            let scope = active_scope
                .or_else(|| candidate.map(|candidate| candidate.scope))
                .ok_or(Error::OutcomeUnknown)?;
            let saved = scope.check(grants, slot, peer, packet.context, now, right)?;

            if let Some(current) = active
                && current.scope.id == id
            {
                return self.active_request(current, packet);
            }

            let candidate = candidate.ok_or(Error::OutcomeUnknown)?;
            if packet.op == lifecycle::CANCEL {
                return self.cancel_queued(candidate.scope, caller, saved, packet.context);
            }

            let ticket_index = self
                .tickets
                .iter()
                .position(|ticket| ticket.is_some_and(|ticket| ticket.id == id));

            if packet.op == a::OBSERVE {
                return super::observation::reply(self.observe(candidate, ticket_index), packet);
            }

            if packet.op == a::SCHEDULE {
                if candidate.record.state != RecordState::Admitted {
                    return Err(Error::Unavailable);
                }
                let index = match ticket_index {
                    Some(index) => index,
                    None => {
                        let free = self
                            .tickets
                            .iter()
                            .position(Option::is_none)
                            .ok_or(Error::Busy)?;
                        self.tickets[free] = Some(Ticket7 {
                            id,
                            caller,
                            saved,
                            subject: candidate.record.subject,
                            stop: false,
                            kind: TicketKind7::Execute,
                        });
                        free
                    }
                };
                return super::observation::activity(
                    candidate.scope,
                    self.tickets[index].unwrap(),
                    packet.op,
                    packet.context,
                );
            }

            let index = ticket_index.ok_or(Error::Unavailable)?;
            let ticket = self.tickets[index].as_mut().unwrap();
            if packet.op == a::REQUEST_CANCEL {
                ticket.stop = true;
            }
            super::observation::activity(candidate.scope, *ticket, packet.op, packet.context)
        })();
        result.unwrap_or_else(|error| failure(packet, error))
    }

    fn active_request(&mut self, active: &mut Active7, packet: Packet) -> Result<Packet, Error> {
        match packet.op {
            a::ACTIVITY => active.packet(packet.op, packet.context),
            a::REQUEST_CANCEL => {
                active.stop();
                if let Some(ticket) = self
                    .tickets
                    .iter_mut()
                    .flatten()
                    .find(|ticket| ticket.id == active.scope.id)
                {
                    ticket.stop = true;
                }
                active.packet(packet.op, packet.context)
            }
            a::SCHEDULE => active.packet(packet.op, packet.context),
            a::OBSERVE => super::observation::reply(
                rustic_abi::files::admission::ObservationV2::Active(active.observation()),
                packet,
            ),
            lifecycle::CANCEL => {
                let already = active.stopping();
                active.stop();
                if let Some(ticket) = self
                    .tickets
                    .iter_mut()
                    .flatten()
                    .find(|ticket| ticket.id == active.scope.id)
                {
                    ticket.stop = true;
                }
                CancelAck {
                    id: active.scope.id,
                    disposition: if already {
                        Disposition::AlreadyRequested
                    } else {
                        Disposition::Requested
                    },
                }
                .packet(packet.context)
            }
            _ => Err(Error::Unsupported),
        }
    }
}

fn failure(packet: Packet, error: Error) -> Packet {
    let mut reply = Packet::new(packet.op);
    reply.context = packet.context;
    reply.status = error as u8;
    reply
}
