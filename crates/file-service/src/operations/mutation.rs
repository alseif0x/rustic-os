// SPDX-License-Identifier: Apache-2.0
use crate::{Grant, Server};
use rustic_abi::files::{operation::Replacement, *};
impl Server {
    pub(super) fn operation_mutation(
        &mut self,
        slot: usize,
        grant: Grant,
        p: Packet,
    ) -> Result<Packet, Error> {
        let mut response = Packet::new(p.op);
        response.context = p.context;
        if p.op == REPLACE_OPEN {
            let request = Replacement::decode(&p)?;
            self.operation_authorize(grant, request)?;
            self.clients.transfers.begin(slot, &p)?;
            return Ok(response);
        }
        let request = self
            .clients
            .transfers
            .logical(slot, false)
            .ok_or(Error::NoTransfer)?;
        if request.resource.object() != p.id {
            return Err(Error::NoTransfer);
        }
        self.operation_authorize(grant, request)?;
        match p.op {
            REPLACE_CHUNK => self.clients.transfers.chunk(slot, &p)?,
            REPLACE_ABORT => self.clients.transfers.clear(slot),
            _ => return Err(Error::Protocol),
        }
        Ok(response)
    }
}
