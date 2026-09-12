// SPDX-License-Identifier: Apache-2.0
use super::codec;
use crate::{ActiveExecution, Caller, Clients, Server, reply};
use rustic_abi::files::{admission as a, *};
use rustic_fs::PollDisk;

impl Server {
    /// Only explicit EXECUTE enters this controller. Live control cannot start
    /// another publication or turn a query into execution.
    #[inline(never)]
    pub fn admission_execute_active(
        &mut self,
        disk: &mut impl PollDisk,
        caller: Caller,
        p: Packet,
        now: u64,
        control: impl FnMut(&mut Clients, &mut ActiveExecution) -> u64,
    ) -> Packet {
        let result = (|| {
            crate::validation::request(&p)?;
            if p.op != a::EXECUTE || caller.context != p.context {
                return Err(Error::Protocol);
            }
            let grant = caller.check(&self.clients, now)?;
            let result = self.execute_admission_active_with(
                disk,
                caller,
                codec::id(a::AdmissionId::decode(&p)?),
                now,
                control,
            )?;
            codec::status(
                self.volume
                    .admission_by_id(grant.subject, result.id)
                    .map_err(reply::error)?,
                p.op,
                p.context,
            )
        })();
        result.unwrap_or_else(|error| {
            let mut r = Packet::new(p.op);
            r.context = p.context;
            r.status = error as u8;
            r
        })
    }
}
