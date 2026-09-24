// SPDX-License-Identifier: Apache-2.0
//! Profile-2 tracked replacement for large workspace files. The caller supplies
//! the file incrementally, so neither side buffers the whole file; the client
//! keeps one 40-byte packet and a running SHA-256 to verify the receipt.
use super::Client;
use rustic_abi::files::{
    operation::{self, OperationId},
    workspace::{Lookup, MAX_FILE_BYTES, Operation, RECEIPT_BYTES, Replacement},
    *,
};
use sha2::{Digest, Sha256};

impl<P: crate::rpc::Progress> Client<P> {
    /// Replace one file with `size` bytes produced by `fill(offset, buffer)`,
    /// which must fill `buffer` with the bytes at `offset`.
    ///
    /// Retain the workspace, retry epoch and key before calling: a failure at
    /// or after commit is [`Error::Uncertain`], and repeating the call with the
    /// same identity and bytes replays the retained receipt without a second
    /// effect. A failure before commit aborts the service transfer.
    pub fn workspace_replace(
        &mut self,
        request: operation::Replacement,
        size: u32,
        mut fill: impl FnMut(u32, &mut [u8]) -> Result<(), Error>,
    ) -> Result<Operation, Error> {
        if size > MAX_FILE_BYTES {
            return Err(Error::Size);
        }
        let open = Replacement { request }.packet(size as usize, self.context)?;
        empty_ack(self.operation_exchange(open)?)?;
        let object = request.resource.object();
        let mut digest = Sha256::new();
        let staged = (|| {
            let mut offset = 0;
            while offset < size {
                let count = (size - offset).min(DATA as u32) as usize;
                let mut p = Packet::new(REPLACE_CHUNK);
                p.id = object;
                p.arg = offset;
                p.count = count as u8;
                fill(offset, &mut p.data[..count])?;
                digest.update(&p.data[..count]);
                empty_ack(self.operation_exchange(p)?)?;
                offset += count as u32;
            }
            Ok(())
        })();
        if let Err(error) = staged {
            let mut p = Packet::new(REPLACE_ABORT);
            p.id = object;
            let _ = self.operation_exchange(p);
            return Err(error);
        }
        let mut p = Packet::new(REPLACE_COMMIT);
        p.id = object;
        let first = self.operation_exchange(p)?;
        let result = self
            .workspace_receipt(first, request.workspace.lineage())
            .map_err(|_| Error::Uncertain)?;
        let sha256: [u8; 32] = digest.finalize().into();
        if result.workspace != request.workspace
            || result.resource != request.resource
            || result.retry != request.retry
            || result.previous_version != request.expected_version
            || result.size != size
            || result.sha256 != sha256
        {
            return Err(Error::Uncertain);
        }
        Ok(result)
    }

    /// Collect the remaining receipt parts after the commit reply.
    fn workspace_receipt(&mut self, first: Packet, lineage: [u8; 16]) -> Result<Operation, Error> {
        let id = OperationId::new(lineage, first.version).map_err(|_| Error::Protocol)?;
        let mut bytes = [0; RECEIPT_BYTES];
        for offset in [0, 40, 80] {
            let p = if offset == 0 {
                first
            } else {
                let mut p = Lookup {
                    query: operation::Lookup::Id(id),
                }
                .packet(self.context);
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
}

fn empty_ack(p: Packet) -> Result<(), Error> {
    if p.id != 0 || p.arg != 0 || p.version != 0 || p.count != 0 {
        return Err(Error::Protocol);
    }
    Ok(())
}
