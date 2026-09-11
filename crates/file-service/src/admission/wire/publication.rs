// SPDX-License-Identifier: Apache-2.0
use super::codec;
use crate::{Caller, Clients, Server, reply};
use rustic_abi::files::{admission as a, *};
use rustic_fs::PollDisk;

impl Server {
    /// Trusted transport supplies caller identity and bounded owner control.
    /// Public cancellation is explicitly scheduled, not in-flight preemption.
    #[inline(never)]
    pub fn admission_with(
        &mut self,
        disk: &mut impl PollDisk,
        caller: Caller,
        p: Packet,
        mut control: impl FnMut(&mut Clients, bool) -> u64,
    ) -> Packet {
        let result = (|| {
            crate::validation::request(&p)?;
            if !a::controlled(p.op) || caller.context != p.context {
                return Err(Error::Protocol);
            }
            let now = control(&mut self.clients, false);
            let right = if p.op == a::CANCEL {
                CANCEL_RIGHT
            } else {
                INSPECT_RIGHT
            };
            let grant = caller.check_right(&self.clients, now, right)?;
            let result = match p.op {
                a::ACCEPT => {
                    let request = self
                        .clients
                        .transfers
                        .logical(caller.slot, true)
                        .ok_or(Error::NoTransfer)?;
                    if request.resource.object() != p.id {
                        return Err(Error::NoTransfer);
                    }
                    self.admission_authorize(grant, codec::stored(request))?;
                    let transfer = self.clients.transfers.take(caller.slot, &p)?;
                    self.admit_with(
                        disk,
                        caller,
                        codec::stored(request),
                        &transfer.data[..transfer.total],
                        &mut control,
                    )?
                }
                a::EXECUTE => self.execute_admission_with(
                    disk,
                    caller,
                    codec::id(a::AdmissionId::decode(&p)?),
                    &mut control,
                )?,
                a::CANCEL => self.cancel_admission_with(
                    disk,
                    caller,
                    codec::id(a::AdmissionId::decode(&p)?),
                    &mut control,
                )?,
                _ => return Err(Error::Protocol),
            };
            // Controllers have checked current authority through final settlement.
            // Only immutable minimal status is disclosed to a CANCEL-only caller.
            codec::status(
                self.volume
                    .admission_by_id(grant.subject, result.id)
                    .map_err(reply::error)?,
                p.op,
                p.context,
            )
        })();
        result.unwrap_or_else(|error| {
            let mut response = Packet::new(p.op);
            response.context = p.context;
            response.status = error as u8;
            response
        })
    }
}
