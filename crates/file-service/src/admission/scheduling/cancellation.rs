// SPDX-License-Identifier: Apache-2.0
//! Serialize stop acceptance under the caller's already checked CANCEL scope.
use super::{Candidate, ExecutionQueue, Ticket};
use crate::{ActiveExecution, Caller};
use rustic_abi::files::{
    Error,
    lifecycle::{CancelAck, Disposition},
};
use rustic_fs::AdmissionState;

impl ExecutionQueue {
    pub(super) fn cancel(
        &mut self,
        candidate: Candidate,
        active: Option<&mut ActiveExecution>,
        caller: Caller,
    ) -> Result<CancelAck, Error> {
        let id = candidate.scope.id;
        let disposition = if candidate.status.state != AdmissionState::Admitted {
            Disposition::TooLate
        } else if let Some(index) = self
            .tickets
            .iter()
            .position(|t| t.is_some_and(|t| t.id == id))
        {
            let ticket = self.tickets[index].as_mut().unwrap();
            let current = if index == 0 && self.running {
                let current = active.ok_or(Error::Busy)?;
                if current.observation().id != id {
                    return Err(Error::Protocol);
                }
                Some(current)
            } else {
                None
            };
            let already = ticket.stop || current.as_ref().is_some_and(|c| c.stopping());
            ticket.stop = true;
            if let Some(current) = current {
                current.stop();
            }
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
            // A prepared admission needs only a prevention ticket, never WRITE
            // authority. run_scheduled checks stop before considering execution.
            // This acceptance is volatile; restart must discard the ticket.
            self.tickets[free] = Some(Ticket {
                id,
                caller,
                subject: candidate.subject,
                stop: true,
            });
            Disposition::Requested
        };
        Ok(CancelAck { id, disposition })
    }
}
