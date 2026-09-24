// SPDX-License-Identifier: Apache-2.0
//! Profile-2 tracked replacement for large workspace files and lookups of their
//! retained receipts. The caller supplies the file incrementally, so neither
//! side buffers the whole file; the client keeps one 40-byte packet and a
//! running SHA-256 to verify the receipt.
use super::Client;
use rustic_abi::files::{
    operation::{self, OperationId},
    workspace::{Lookup, MAX_FILE_BYTES, Operation, RECEIPT_BYTES, Replacement},
    *,
};
use sha2::{Digest, Sha256};

/// One profile-2 transfer opened on the client's current binding.
///
/// It carries no authority of its own: it only remembers the request, the next
/// offset and the running SHA-256 of the acknowledged bytes. If the binding is
/// revoked or replaced, the service has dropped the transfer and refuses the
/// next chunk (`Closed` on the old endpoint, `NoTransfer` on a new binding).
pub struct WorkspaceTransfer {
    request: operation::Replacement,
    size: u32,
    offset: u32,
    digest: Sha256,
}

impl WorkspaceTransfer {
    /// Bytes the service has acknowledged so far.
    pub fn offset(&self) -> u32 {
        self.offset
    }

    /// Total size announced when the transfer was opened.
    pub fn size(&self) -> u32 {
        self.size
    }
}

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
        let mut transfer = self.workspace_open(request, size)?;
        while transfer.offset < transfer.size {
            if let Err(error) = self.workspace_chunk(&mut transfer, &mut fill) {
                let _ = self.workspace_abort(transfer);
                return Err(error);
            }
        }
        self.workspace_commit(transfer)
    }

    /// Open a profile-2 transfer of `size` bytes. Nothing is committed until
    /// [`Self::workspace_commit`]; abandon it with [`Self::workspace_abort`].
    pub fn workspace_open(
        &mut self,
        request: operation::Replacement,
        size: u32,
    ) -> Result<WorkspaceTransfer, Error> {
        if size > MAX_FILE_BYTES {
            return Err(Error::Size);
        }
        let open = Replacement { request }.packet(size as usize, self.context)?;
        empty_ack(self.operation_exchange(open)?)?;
        Ok(WorkspaceTransfer {
            request,
            size,
            offset: 0,
            digest: Sha256::new(),
        })
    }

    /// Send the next chunk of at most 40 bytes, produced by `fill(offset,
    /// buffer)`. The offset advances, and the bytes count towards the receipt
    /// check, only when the service acknowledges them.
    pub fn workspace_chunk(
        &mut self,
        transfer: &mut WorkspaceTransfer,
        fill: impl FnOnce(u32, &mut [u8]) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if transfer.offset >= transfer.size {
            return Err(Error::Offset);
        }
        let count = (transfer.size - transfer.offset).min(DATA as u32) as usize;
        let mut p = Packet::new(REPLACE_CHUNK);
        p.id = transfer.request.resource.object();
        p.arg = transfer.offset;
        p.count = count as u8;
        fill(transfer.offset, &mut p.data[..count])?;
        empty_ack(self.operation_exchange(p)?)?;
        transfer.digest.update(&p.data[..count]);
        transfer.offset += count as u32;
        Ok(())
    }

    /// Release an open transfer on the service without committing it.
    pub fn workspace_abort(&mut self, transfer: WorkspaceTransfer) -> Result<(), Error> {
        let mut p = Packet::new(REPLACE_ABORT);
        p.id = transfer.request.resource.object();
        empty_ack(self.operation_exchange(p)?)
    }

    /// Commit a complete transfer and verify its receipt against the request
    /// and the acknowledged bytes. An incomplete transfer is aborted and
    /// refused with [`Error::Offset`]. Any failure after the commit request is
    /// sent is [`Error::Uncertain`]; see [`Self::workspace_replace`].
    pub fn workspace_commit(&mut self, transfer: WorkspaceTransfer) -> Result<Operation, Error> {
        if transfer.offset != transfer.size {
            let _ = self.workspace_abort(transfer);
            return Err(Error::Offset);
        }
        let WorkspaceTransfer {
            request,
            size,
            digest,
            ..
        } = transfer;
        let mut p = Packet::new(REPLACE_COMMIT);
        p.id = request.resource.object();
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

    /// Look up a retained profile-2 receipt by operation ID or by retry
    /// identity. It needs the inspect right and only finds operations of the
    /// grant's own subject and scope; the receipt's SHA-256 is the service's
    /// digest of the retained bytes. A reply that names another operation is
    /// [`Error::Protocol`].
    pub fn workspace_operation(&mut self, query: operation::Lookup) -> Result<Operation, Error> {
        let lineage = match query {
            operation::Lookup::Retry { workspace, .. } => workspace.lineage(),
            operation::Lookup::Id(id) => id.lineage(),
        };
        let first = self.operation_exchange(Lookup { query }.packet(self.context))?;
        let result = self.workspace_receipt(first, lineage)?;
        let matches = match query {
            operation::Lookup::Retry { workspace, retry } => {
                result.workspace == workspace && result.retry == retry
            }
            operation::Lookup::Id(id) => result.id == id,
        };
        if !matches {
            return Err(Error::Protocol);
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
