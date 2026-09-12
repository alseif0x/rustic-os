// SPDX-License-Identifier: Apache-2.0
//! One publication per dispatch pass, with fresh guards and service-owned prevention.
use super::ExecutionQueue;
use crate::{ActiveExecution, Clients, Server, reply};
use rustic_abi::files::Error;
use rustic_fs::{AdmissionId, AdmissionState, AdmissionStatus, PollDisk};

impl Server {
    /// Drives at most one queued record. The callback services bounded owner and
    /// public traffic, including the initiating client and other retained work.
    /// No terminal reply belongs to the scheduling request; clients query durable
    /// admission/operation facts. An uncertain volume abandons volatile schedules.
    #[inline(never)]
    pub fn run_scheduled(
        &mut self,
        disk: &mut impl PollDisk,
        queue: &mut ExecutionQueue,
        now: u64,
        mut control: impl FnMut(&mut Clients, &mut ActiveExecution, &mut ExecutionQueue) -> u64,
    ) -> Option<Result<AdmissionStatus, Error>> {
        let ticket = queue.tickets[0]?;
        let id = AdmissionId {
            lineage: ticket.id.lineage(),
            number: ticket.id.number(),
        };
        let result = (|| {
            queue.refresh(self)?;
            let old = self
                .volume
                .admission_by_id(ticket.subject, id)
                .map_err(reply::error)?;
            if old.status.state != AdmissionState::Admitted {
                return Ok(old.status);
            }
            let mut prevention = ActiveExecution::new(self, ticket.subject, old)?;
            queue.running = true;
            if !ticket.stop {
                let result = self.execute_admission_active_with(
                    disk,
                    ticket.caller,
                    id,
                    now,
                    |clients, active| {
                        if queue.tickets[0].is_some_and(|t| t.stop) {
                            active.stop();
                        }
                        control(clients, active, queue)
                    },
                );
                match result {
                    // A queued operation may lose its guard before any data is
                    // admitted. Persist prevention rather than silently leave it
                    // scheduled or run it under another client's authority.
                    Err(
                        Error::Version
                        | Error::Denied
                        | Error::Revoked
                        | Error::Expired
                        | Error::OutcomeUnknown,
                    ) => (),
                    result => return result,
                }
            }
            self.prevent_admission_active(
                disk,
                ticket.subject,
                id,
                &mut prevention,
                &mut |clients, active| control(clients, active, queue),
            )
        })();
        if matches!(result, Err(Error::Uncertain)) || self.volume.stat(1).is_err() {
            queue.clear();
        } else {
            queue.finish();
        }
        Some(result)
    }
}
