// SPDX-License-Identifier: Apache-2.0
//! Queue admission and observation under live, separately scoped rights.
use super::{ExecutionQueue, Ticket};
use crate::{ActiveExecution, Caller, Clients, Server};
use rustic_abi::files::lifecycle::{self, CancelAck};
use rustic_abi::files::{CANCEL_RIGHT, Error, INSPECT_RIGHT, Packet, WRITE_RIGHT, admission as a};
use rustic_fs::AdmissionState;

impl Server {
    /// Ordinary boundary only. Refreshes namespace proofs before replying;
    /// callers must service this queue through run_scheduled to start work.
    pub fn scheduling_request(
        &self,
        queue: &mut ExecutionQueue,
        caller: Caller,
        p: Packet,
        now: u64,
    ) -> Packet {
        if p.op == a::OBSERVE
            && let Err(error) = self.volume.retained_admission(0)
        {
            return failure(p, crate::reply::error(error));
        }
        match queue.refresh(self) {
            Ok(()) => queue.request(&self.clients, None, caller, p, now),
            Err(error) => failure(p, error),
        }
    }
}
impl ExecutionQueue {
    /// Active transport may pass the current execution. No storage calls, grant
    /// issuance or namespace changes occur through this restricted interface.
    pub fn request(
        &mut self,
        clients: &Clients,
        mut active: Option<&mut ActiveExecution>,
        caller: Caller,
        p: Packet,
        now: u64,
    ) -> Packet {
        let result = (|| {
            crate::validation::request(&p)?;
            if !(a::live(p.op) || matches!(p.op, a::SCHEDULE | a::OBSERVE | lifecycle::CANCEL))
                || caller.context != p.context
            {
                return Err(Error::Protocol);
            }
            let right = match p.op {
                a::SCHEDULE => INSPECT_RIGHT | WRITE_RIGHT,
                a::REQUEST_CANCEL | lifecycle::CANCEL => CANCEL_RIGHT,
                _ => INSPECT_RIGHT,
            };
            caller.check_right(clients, now, right)?;
            let id = if p.op == lifecycle::CANCEL {
                CancelAck::decode_request(&p)?
            } else {
                a::AdmissionId::decode(&p)?
            };
            let candidate = self
                .candidates
                .iter()
                .flatten()
                .find(|c| c.scope.id == id)
                .ok_or(
                    if active.is_some() || matches!(p.op, a::OBSERVE | lifecycle::CANCEL) {
                        Error::OutcomeUnknown
                    } else {
                        Error::Unavailable
                    },
                )?;
            candidate.scope.check(clients, caller, right, now)?;
            if p.op == lifecycle::CANCEL {
                return self.cancel(*candidate, active, caller)?.packet(p.context);
            }
            let index = self
                .tickets
                .iter()
                .position(|t| t.is_some_and(|t| t.id == id));
            if p.op == a::OBSERVE {
                return super::super::observation::reply(
                    self.observe(candidate, active.as_deref())?,
                    p,
                );
            }
            if p.op == a::SCHEDULE && index.is_none() {
                if candidate.status.state != AdmissionState::Admitted {
                    return Err(Error::Unavailable);
                }
                let free = self
                    .tickets
                    .iter()
                    .position(Option::is_none)
                    .ok_or(Error::Busy)?;
                self.tickets[free] = Some(Ticket {
                    id,
                    caller,
                    subject: candidate.subject,
                    stop: false,
                });
            }
            let index = self
                .tickets
                .iter()
                .position(|t| t.is_some_and(|t| t.id == id))
                .ok_or(Error::Unavailable)?;
            let ticket = self.tickets[index].as_mut().unwrap();
            if p.op == a::REQUEST_CANCEL {
                ticket.stop = true;
            }
            if index == 0 && self.running {
                let current = active.as_mut().ok_or(Error::Busy)?;
                if current.observation().id != id {
                    return Err(Error::Protocol);
                }
                if ticket.stop {
                    current.stop();
                    // The active controller will observe this latch before advancing.
                }
                let mut view = current.observation();
                if view.cancel_requested && view.phase == a::ActivityPhase::Running {
                    view.phase = a::ActivityPhase::Stopping;
                }
                return view.packet(p.op, p.context);
            }
            a::Activity {
                id,
                service_instance: candidate.scope.instance,
                phase: a::ActivityPhase::Queued,
                cancel_requested: ticket.stop,
                io_pending: false,
            }
            .packet(p.op, p.context)
        })();
        result.unwrap_or_else(|error| failure(p, error))
    }
}
fn failure(p: Packet, error: Error) -> Packet {
    let mut reply = Packet::new(p.op);
    reply.context = p.context;
    reply.status = error as u8;
    reply
}
