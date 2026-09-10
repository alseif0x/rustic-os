// SPDX-License-Identifier: Apache-2.0
use crate::{Grant, Server, reply};
use rustic_abi::files::{operation::Replacement, *};
use rustic_fs::Disk;
impl Server {
    pub(super) fn operation_mutation(
        &mut self,
        disk: &mut impl Disk,
        slot: usize,
        grant: Grant,
        p: Packet,
    ) -> Result<Packet, Error> {
        let mut response = Packet::new(p.op);
        response.context = p.context;
        if p.op == REPLACE_OPEN {
            let request = Replacement::decode(&p)?;
            self.operation_authorize(grant, request)?;
            self.transfers.begin(slot, &p)?;
            return Ok(response);
        }
        let request = self.transfers.logical(slot).ok_or(Error::NoTransfer)?;
        if request.resource.object() != p.id {
            return Err(Error::NoTransfer);
        }
        let new_operation = self.operation_authorize(grant, request)?;
        match p.op {
            REPLACE_CHUNK => self.transfers.chunk(slot, &p)?,
            REPLACE_ABORT => self.transfers.clear(slot),
            REPLACE_COMMIT => {
                let transfer = self.transfers.take(slot, &p)?;
                let r = self
                    .volume
                    .replace_scoped(
                        disk,
                        grant.subject,
                        self.instance,
                        rustic_fs::Replacement {
                            workspace: request.workspace.root(),
                            retry: super::query::stored(request.workspace, request.retry),
                            id: request.resource.object(),
                            version: request.expected_version.value(),
                        },
                        &transfer.data[..transfer.total],
                    )
                    .map_err(reply::error)?;
                if new_operation && self.instance == 0 {
                    self.instance = r.committed;
                }
                let old = self
                    .volume
                    .operation_by_id(grant.subject, r.retry.lineage, r.committed)
                    .map_err(reply::error)?;
                response = super::query::receipt(old)?.part(p.op, p.context, 0)?;
            }
            _ => return Err(Error::Protocol),
        }
        Ok(response)
    }
}
