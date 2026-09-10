// SPDX-License-Identifier: Apache-2.0
//! Logical completed operations; storage evidence, policy and wire framing stay separate.
mod authority;
mod mutation;
mod query;
use crate::{Grant, Server};
use rustic_abi::files::*;
use rustic_fs::Disk;
impl Server {
    pub(super) fn operation_request(
        &mut self,
        disk: &mut impl Disk,
        slot: usize,
        grant: Grant,
        p: Packet,
    ) -> Result<Packet, Error> {
        if matches!(p.op, OPERATION_RETRY | OPERATION_ID | OPERATION_PART) {
            return self.operation_query(grant, p);
        }
        self.operation_mutation(disk, slot, grant, p)
    }
}
