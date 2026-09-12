// SPDX-License-Identifier: Apache-2.0
//! Restricted authority view while publication exclusively owns volume and disk.
use super::{Caller, scope::Scope};
use crate::{Clients, Server};
use rustic_abi::files::{CANCEL_RIGHT, Error, INSPECT_RIGHT, Packet, admission as a};
use rustic_fs::{Admission, PublicationPhase};

pub struct ActiveExecution {
    observation: a::Activity,
    scope: Scope,
}
impl ActiveExecution {
    pub(super) fn new(server: &Server, subject: u64, old: Admission<'_>) -> Result<Self, Error> {
        let scope = Scope::new(server, subject, &old)?;
        Ok(Self {
            observation: a::Activity {
                id: scope.id,
                service_instance: scope.instance,
                phase: a::ActivityPhase::Running,
                cancel_requested: false,
                io_pending: false,
            },
            scope,
        })
    }
    pub(super) fn observation(&self) -> a::Activity {
        self.observation
    }
    pub fn pending(&self) -> bool {
        self.observation.io_pending
    }
    pub(super) fn stopping(&self) -> bool {
        self.observation.cancel_requested
    }
    pub(super) fn stop(&mut self) {
        self.observation.cancel_requested = true;
    }
    pub(super) fn observe(&mut self, phase: PublicationPhase, pending: bool, cleanup: bool) {
        self.observation.io_pending = pending;
        self.observation.phase = if cleanup {
            a::ActivityPhase::Stopping
        } else if matches!(
            phase,
            PublicationPhase::Settling | PublicationPhase::Committed
        ) {
            a::ActivityPhase::Settling
        } else if self.stopping() {
            a::ActivityPhase::Stopping
        } else {
            a::ActivityPhase::Running
        };
    }
    /// Transport binds slot/peer. The immutable scope proof was captured before
    /// borrowing storage. Clients permits only expiry/revoke/detach during this
    /// borrow; no issuance or namespace mutation can invalidate the scope proof.
    /// Still check the live binding, right and deadline on EVERY request.
    pub fn request(&mut self, clients: &Clients, caller: Caller, p: Packet, now: u64) -> Packet {
        let result = (|| {
            crate::validation::request(&p)?;
            if !a::live(p.op) || caller.context != p.context {
                return Err(Error::Protocol);
            }
            let right = if p.op == a::REQUEST_CANCEL {
                CANCEL_RIGHT
            } else {
                INSPECT_RIGHT
            };
            self.scope.check(clients, caller, right, now)?;
            if a::AdmissionId::decode(&p)? != self.observation.id {
                return Err(Error::OutcomeUnknown);
            }
            if p.op == a::REQUEST_CANCEL {
                self.stop();
                if self.observation.phase == a::ActivityPhase::Running {
                    self.observation.phase = a::ActivityPhase::Stopping;
                }
            }
            self.observation.packet(p.op, p.context)
        })();
        result.unwrap_or_else(|error| {
            let mut response = Packet::new(p.op);
            response.context = p.context;
            response.status = error as u8;
            response
        })
    }
}
