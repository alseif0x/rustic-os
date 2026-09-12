// SPDX-License-Identifier: Apache-2.0
//! Incarnation-local FIFO over already durable records, never a source of authority.
mod dispatch;
mod execution;
mod inventory;
use super::{Caller, scope::Scope};
use rustic_abi::files::admission::AdmissionId;
use rustic_fs::{AdmissionState, RETAINED};

#[derive(Clone, Copy)]
struct Candidate {
    scope: Scope,
    subject: u64,
    state: AdmissionState,
}
#[derive(Clone, Copy)]
struct Ticket {
    id: AdmissionId,
    caller: Caller,
    subject: u64,
    stop: bool,
}

/// The transport owns one queue for one Server incarnation. It must drop the
/// queue on restart and call run_scheduled after bounded ordinary dispatch.
/// Snapshots are refreshed by Server before use; only active control may use
/// them while publication excludes namespace changes and new grant issuance.
pub struct ExecutionQueue {
    candidates: [Option<Candidate>; RETAINED],
    tickets: [Option<Ticket>; RETAINED],
    running: bool,
}
impl Default for ExecutionQueue {
    fn default() -> Self {
        Self::new()
    }
}
impl ExecutionQueue {
    pub const fn new() -> Self {
        Self {
            candidates: [None; RETAINED],
            tickets: [None; RETAINED],
            running: false,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.tickets[0].is_none()
    }
    fn finish(&mut self) {
        self.tickets.rotate_left(1);
        self.tickets[RETAINED - 1] = None;
        self.running = false;
    }
    fn clear(&mut self) {
        self.tickets = [None; RETAINED];
        self.candidates = [None; RETAINED];
        self.running = false;
    }
}
