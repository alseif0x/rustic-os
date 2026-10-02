// SPDX-License-Identifier: Apache-2.0
//! Accept volatile stops under the caller's existing CANCEL scope.
use super::super::scope::Scope7;
use super::{Caller7, Scheduler7, Ticket7, TicketKind7};
use rustic_abi::files::{
    Error,
    lifecycle::{CancelAck, Disposition},
};
use rustic_fs::format7::RecordState;

impl Scheduler7 {
    pub(super) fn cancel_queued(
        &mut self,
        scope: Scope7,
        caller: Caller7,
        saved: crate::v7::Grant7,
        context: u32,
    ) -> Result<rustic_abi::files::Packet, Error> {
        let disposition = if scope.terminal_state() {
            Disposition::TooLate
        } else if let Some(index) = self
            .tickets
            .iter()
            .position(|ticket| ticket.is_some_and(|ticket| ticket.id == scope.id))
        {
            let ticket = self.tickets[index].as_mut().unwrap();
            let already = ticket.stop;
            ticket.stop = true;
            if already {
                Disposition::AlreadyRequested
            } else {
                Disposition::Requested
            }
        } else {
            let free = self
                .tickets
                .iter()
                .position(Option::is_none)
                .ok_or(Error::Busy)?;
            // The accepted stop owns only a CANCEL-authorized prevention. It
            // never receives execution or WRITE authority.
            self.tickets[free] = Some(Ticket7 {
                id: scope.id,
                caller,
                saved,
                subject: scope.subject,
                stop: true,
                kind: TicketKind7::Prevent,
            });
            debug_assert_eq!(scope_status(scope), RecordState::Admitted);
            Disposition::Requested
        };
        CancelAck {
            id: scope.id,
            disposition,
        }
        .packet(context)
    }
}

fn scope_status(scope: Scope7) -> RecordState {
    match scope.status.state {
        rustic_abi::files::admission::State::Admitted => RecordState::Admitted,
        rustic_abi::files::admission::State::Cancelled => RecordState::Cancelled,
        rustic_abi::files::admission::State::Committed => RecordState::AdmittedCommitted,
    }
}
