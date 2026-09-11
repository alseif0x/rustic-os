// SPDX-License-Identifier: Apache-2.0
use super::codec;
use crate::{Grant, Server, reply};
use rustic_abi::files::{
    admission as a,
    operation::{Lookup, Replacement},
    *,
};

impl Server {
    pub(crate) fn admission_request(
        &mut self,
        slot: usize,
        grant: Grant,
        p: Packet,
    ) -> Result<Packet, Error> {
        if grant.subject == 0 || grant.rights & INSPECT_RIGHT == 0 {
            return Err(Error::Denied);
        }
        let mut response = Packet::new(p.op);
        response.context = p.context;
        match p.op {
            a::OPEN => {
                let request = Replacement::decode(&p)?;
                self.admission_authorize(grant, codec::stored(request))?;
                self.clients.transfers.begin(slot, &p)?;
            }
            a::CHUNK | a::ABORT => {
                let request = self
                    .clients
                    .transfers
                    .logical(slot, true)
                    .ok_or(Error::NoTransfer)?;
                if request.resource.object() != p.id {
                    return Err(Error::NoTransfer);
                }
                self.admission_authorize(grant, codec::stored(request))?;
                if p.op == a::CHUNK {
                    self.clients.transfers.chunk(slot, &p)?;
                } else {
                    self.clients.transfers.clear(slot);
                }
            }
            a::GET | a::RETRY => {
                let old = if p.op == a::GET {
                    self.inspect_admission(grant, codec::id(a::AdmissionId::decode(&p)?))?
                } else {
                    let mut lookup = p;
                    lookup.op = OPERATION_RETRY;
                    let Lookup::Retry { workspace, retry } = Lookup::decode(&lookup)? else {
                        return Err(Error::Protocol);
                    };
                    let old = self
                        .volume
                        .admission_by_retry(
                            grant.subject,
                            workspace.root(),
                            rustic_fs::Retry {
                                lineage: workspace.lineage(),
                                epoch: retry.epoch.value(),
                                key: retry.key.value(),
                            },
                        )
                        .map_err(reply::error)?;
                    grant
                        .operation_inspect(&self.volume, old.request.workspace, old.request.id)
                        .map_err(|_| Error::OutcomeUnknown)?;
                    old
                };
                return codec::status(old, p.op, p.context);
            }
            _ => return Err(Error::Protocol),
        }
        Ok(response)
    }
}
