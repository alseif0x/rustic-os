// SPDX-License-Identifier: Apache-2.0
//! Typed synchronous completion and read-only recovery; no implicit mutation replay.
use super::Client;
use rustic_abi::files::{
    operation::{Lookup, Operation, OperationId, RECEIPT_BYTES, Replacement},
    *,
};
use sha2::{Digest, Sha256};
impl<P: crate::rpc::Progress> Client<P> {
    fn operation_exchange(&mut self, mut p: Packet) -> Result<Packet, Error> {
        self.require_binding()?;
        p.context = self.context;
        let durable = p.op == REPLACE_COMMIT;
        let message = self.rpc.exchange(&p.encode()).map_err(|e| match e {
            _ if durable => Error::Uncertain,
            crate::Error::Interrupted => Error::Interrupted,
            crate::Error::Ipc(crate::abi::ipc::Error::Closed) => Error::Closed,
            _ => Error::Protocol,
        })?;
        Packet::decode(message.payload())
            .and_then(|r| r.checked_reply(p.op, p.context))
            .map_err(|e| {
                if durable && e == Error::Protocol {
                    Error::Uncertain
                } else {
                    e
                }
            })
    }
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
    /// Staging is volatile and has no file effect or durable operation acknowledgement.
    pub fn stage_replace(&mut self, request: Replacement, bytes: &[u8]) -> Result<(), Error> {
        let first = request.packet(bytes.len(), self.context)?;
        let mut opened = false;
        let stage = (|| {
            let ack = self.operation_exchange(first)?;
            if ack.id != 0 || ack.arg != 0 || ack.version != 0 || ack.count != 0 {
                return Err(Error::Protocol);
            }
            opened = true;
            for (index, chunk) in bytes.chunks(DATA).enumerate() {
                let mut p = Packet::new(REPLACE_CHUNK);
                p.id = request.resource.object();
                p.arg = (index * DATA) as u32;
                p.count = chunk.len() as u8;
                p.data[..chunk.len()].copy_from_slice(chunk);
                let ack = self.operation_exchange(p)?;
                if ack.id != 0 || ack.arg != 0 || ack.version != 0 || ack.count != 0 {
                    return Err(Error::Protocol);
                }
            }
            Ok(())
        })();
        if opened && stage.is_err() {
            let mut p = Packet::new(REPLACE_ABORT);
            p.id = request.resource.object();
            let _ = self.operation_exchange(p);
        }
        stage
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
