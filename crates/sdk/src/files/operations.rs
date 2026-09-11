// SPDX-License-Identifier: Apache-2.0
//! Typed synchronous completion and read-only recovery; no implicit mutation replay.
use super::Client;
use rustic_abi::files::{
    operation::{Lookup, Operation, OperationId, RECEIPT_BYTES, Replacement},
    *,
};
use sha2::{Digest, Sha256};
impl<P: crate::rpc::Progress> Client<P> {
    fn collect_operation(&mut self, first: Packet, lineage: [u8; 16]) -> Result<Operation, Error> {
        let id = OperationId::new(lineage, first.version).map_err(|_| Error::Protocol)?;
        let mut bytes = [0; RECEIPT_BYTES];
        for offset in [0, 40, 80] {
            let p = if offset == 0 {
                first
            } else {
                let mut p = Lookup::Id(id).packet(self.context);
                p.op = OPERATION_PART;
                p.arg = offset as u32;
                self.operation_exchange(p)?
            };
            let length = (RECEIPT_BYTES - offset).min(DATA);
            if p.id != offset as u32
                || p.arg != RECEIPT_BYTES as u32
                || p.version != id.sequence()
                || p.count as usize != length
            {
                return Err(Error::Protocol);
            }
            bytes[offset..offset + length].copy_from_slice(p.payload());
        }
        let operation = Operation::decode(&bytes)?;
        if operation.id != id {
            return Err(Error::Protocol);
        }
        Ok(operation)
    }
    pub fn operation_get(&mut self, lookup: Lookup) -> Result<Operation, Error> {
        let lineage = match lookup {
            Lookup::Retry { workspace, .. } => workspace.lineage(),
            Lookup::Id(id) => id.lineage(),
        };
        let first = self.operation_exchange(lookup.packet(self.context))?;
        let result = self.collect_operation(first, lineage)?;
        let matches = match lookup {
            Lookup::Retry { workspace, retry } => {
                result.workspace == workspace && result.retry == retry
            }
            Lookup::Id(id) => result.id == id,
        };
        if !matches {
            return Err(Error::Protocol);
        }
        Ok(result)
    }
    /// Volatile staging for the synchronous completed-operation profile.
    pub fn stage_replace(&mut self, request: Replacement, bytes: &[u8]) -> Result<(), Error> {
        self.stage_profile(request, bytes, false)
    }
    /// Retain workspace/epoch/key before calling. A missing final receipt is
    /// uncertain even if the first fragment arrived; recover via operation_get.
    pub fn replace_file(&mut self, request: Replacement, bytes: &[u8]) -> Result<Operation, Error> {
        self.stage_replace(request, bytes)?;
        let mut p = Packet::new(REPLACE_COMMIT);
        p.id = request.resource.object();
        let first = self.operation_exchange(p)?;
        let result = self
            .collect_operation(first, request.workspace.lineage())
            .map_err(|_| Error::Uncertain)?;
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        if result.workspace != request.workspace
            || result.resource != request.resource
            || result.retry != request.retry
            || result.previous_version != request.expected_version
            || result.size as usize != bytes.len()
            || result.sha256 != digest
        {
            return Err(Error::Uncertain);
        }
        Ok(result)
    }
}
