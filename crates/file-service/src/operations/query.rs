// SPDX-License-Identifier: Apache-2.0
use crate::{Grant, Server, reply};
use rustic_abi::files::{
    operation::{Instance, Key, Lookup, Operation, OperationId, Retry},
    reference::{Epoch, Resource, Version, Workspace},
    *,
};
use sha2::{Digest, Sha256};
pub(super) fn stored(workspace: Workspace, retry: Retry) -> rustic_fs::Retry {
    rustic_fs::Retry {
        lineage: workspace.lineage(),
        epoch: retry.epoch.value(),
        key: retry.key.value(),
    }
}
pub(super) fn receipt(old: rustic_fs::Operation<'_>) -> Result<Operation, Error> {
    let r = old.receipt;
    let workspace = Workspace::new(r.retry.lineage, old.workspace)?;
    Ok(Operation {
        id: OperationId::new(r.retry.lineage, r.committed)?,
        service_instance: Instance::new(r.retry.lineage, old.instance)?,
        workspace,
        resource: Resource::new(workspace, r.id)?,
        previous_version: Version::new(r.previous)?,
        version: Version::new(r.committed)?,
        size: r.length,
        retry: Retry {
            epoch: Epoch::new(r.retry.epoch)?,
            key: Key::new(r.retry.key)?,
        },
        sha256: Sha256::digest(old.bytes).into(),
    })
}
impl Server {
    pub(super) fn operation_query(&self, grant: Grant, p: Packet) -> Result<Packet, Error> {
        if grant.subject == 0 || grant.rights & INSPECT_RIGHT == 0 {
            return Err(Error::Denied);
        }
        let old = match Lookup::decode(&p)? {
            Lookup::Retry { workspace, retry } => self.volume.operation_by_retry(
                grant.subject,
                workspace.root(),
                stored(workspace, retry),
            ),
            Lookup::Id(id) => {
                self.volume
                    .operation_by_id(grant.subject, id.lineage(), id.sequence())
            }
        }
        .map_err(reply::error)?;
        // Missing and out-of-scope records have the same observation result.
        // Otherwise a guessed operation ID could reveal another scope's history.
        grant
            .operation_inspect(&self.volume, old.workspace, old.receipt.id)
            .map_err(|_| Error::OutcomeUnknown)?;
        receipt(old)?.part(
            p.op,
            p.context,
            if p.op == OPERATION_PART {
                p.arg as usize
            } else {
                0
            },
        )
    }
}
