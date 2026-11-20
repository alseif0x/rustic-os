// SPDX-License-Identifier: Apache-2.0
//! Bounded volatile FIFO for durable V7 admissions.
mod cancellation;
mod dispatch;
pub(in crate::v7) mod execution;
mod inventory;
mod observation;

use super::super::Grant7;
use super::scope::Scope7;
use rustic_abi::files::{admission::AdmissionId, lifecycle};
use rustic_fs::format7::{RETAINED, Record7};

pub(super) const TICKETS7: usize = 2;

/// Trusted transport identity, never decoded from a packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::v7) struct Caller7 {
    pub(in crate::v7) slot: usize,
    pub(in crate::v7) peer: u64,
    pub(in crate::v7) context: u32,
}

#[derive(Clone, Copy)]
pub(in crate::v7) struct Candidate7 {
    pub(in crate::v7) scope: Scope7,
    pub(in crate::v7) record: Record7,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::v7) enum TicketKind7 {
    Execute,
    Prevent,
}

#[derive(Clone, Copy)]
pub(in crate::v7) struct Ticket7 {
    pub(in crate::v7) id: AdmissionId,
    pub(in crate::v7) caller: Caller7,
    pub(in crate::v7) saved: Grant7,
    pub(in crate::v7) subject: u64,
    pub(in crate::v7) stop: bool,
    pub(in crate::v7) kind: TicketKind7,
}

/// Server-incarnation-local queue and bounded retained inventory. Restarting
/// a V7 Server drops both; nothing resumes automatically from durable records.
pub(in crate::v7) struct Scheduler7 {
    candidates: [Option<Candidate7>; RETAINED],
    tickets: [Option<Ticket7>; TICKETS7],
}

impl Scheduler7 {
    pub(in crate::v7) const fn new() -> Self {
        Self {
            candidates: [None; RETAINED],
            tickets: [None; TICKETS7],
        }
    }

    pub(in crate::v7) fn has_scheduled(&self) -> bool {
        self.tickets.iter().any(Option::is_some)
    }

    pub(in crate::v7) fn contains(&self, id: AdmissionId) -> bool {
        self.tickets.iter().flatten().any(|ticket| ticket.id == id)
    }

    pub(in crate::v7) fn candidate(&self, id: AdmissionId) -> Option<Candidate7> {
        self.candidates
            .iter()
            .flatten()
            .copied()
            .find(|candidate| candidate.scope.id == id)
    }

    pub(in crate::v7) fn front(&self) -> Option<Ticket7> {
        self.tickets[0]
    }

    pub(in crate::v7) fn finish(&mut self) {
        self.tickets.rotate_left(1);
        self.tickets[TICKETS7 - 1] = None;
    }

    pub(in crate::v7) fn clear(&mut self) {
        self.tickets = [None; TICKETS7];
        self.candidates = [None; RETAINED];
    }
}

impl Default for Scheduler7 {
    fn default() -> Self {
        Self::new()
    }
}

pub(in crate::v7) fn is_lifecycle(op: u8) -> bool {
    matches!(
        op,
        rustic_abi::files::admission::ACTIVITY
            | rustic_abi::files::admission::REQUEST_CANCEL
            | rustic_abi::files::admission::SCHEDULE
            | rustic_abi::files::admission::OBSERVE
            | lifecycle::CANCEL
    )
}
