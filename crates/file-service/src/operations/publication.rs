// SPDX-License-Identifier: Apache-2.0
//! Storage settlement with independent, trusted owner control between polls.
use crate::{Clients, Server, reply};
use core::task::Poll;
use rustic_abi::files::*;
use rustic_fs::{Disk, PollDisk, PublicationPhase as Phase};

pub(crate) struct Synchronous<'a, D>(pub(crate) &'a mut D);
impl<D: Disk> PollDisk for Synchronous<'_, D> {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), rustic_fs::Error>> {
        Poll::Ready(self.0.write(sector, bytes))
    }
    fn poll_flush(&mut self) -> Poll<Result<(), rustic_fs::Error>> {
        Poll::Ready(self.0.flush())
    }
}

impl Server {
    /// Drive one completed-profile replacement while trusted control can revoke,
    /// detach or expire clients. The callback must be bounded and return current
    /// monotonic time. It cannot mutate storage or issue new grants. `pending`
    /// permits a bounded wait between polls; it never means an effect is prevented.
    /// No durable admission or public operations.cancel is acknowledged here.
    // Keep the publication/transfer buffers out of unrelated request stack frames.
    #[inline(never)]
    pub fn commit_with(
        &mut self,
        disk: &mut impl PollDisk,
        slot: usize,
        peer: u64,
        p: Packet,
        mut control: impl FnMut(&mut Clients, bool) -> u64,
    ) -> Packet {
        let result = (|| {
            crate::validation::request(&p)?;
            if p.op != REPLACE_COMMIT {
                return Err(Error::Protocol);
            }
            let now = control(&mut self.clients, false);
            let grant = self.grant_at(slot).ok_or(Error::Denied)?;
            grant.check(peer, p.context, now)?;
            let request = self
                .clients
                .transfers
                .logical(slot)
                .ok_or(Error::NoTransfer)?;
            if request.resource.object() != p.id {
                return Err(Error::NoTransfer);
            }
            let new_operation = self.operation_authorize(grant, request)?;
            let transfer = self.clients.transfers.take(slot, &p)?;
            let mut write = self
                .volume
                .prepare_scoped(
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
            let mut denied = None;
            loop {
                let now = control(&mut self.clients, write.pending());
                self.clients.expire(now);
                // Scope/subject/rights cannot change under this borrow: reissuance
                // needs Server + Volume. Every poll rechecks the live peer/context,
                // deadline and revocation before submitting the next command.
                if let Err(error) = self
                    .clients
                    .grant_at(slot)
                    .ok_or(Error::Revoked)
                    .and_then(|current| current.check(peer, p.context, now))
                {
                    denied.get_or_insert(error);
                    write.cancel().map_err(reply::error)?;
                }
                match write.phase() {
                    Phase::Cancelled => return Err(denied.unwrap_or(Error::Revoked)),
                    Phase::Committed => break,
                    _ => (),
                }
                if let Poll::Ready(result) = write.poll_advance() {
                    result.map_err(reply::error)?;
                }
            }
            let receipt = write.result().unwrap();
            drop(write);
            if new_operation && self.instance == 0 {
                self.instance = receipt.committed;
            }
            // A late revocation cannot undo a committed effect. Withhold the
            // receipt under lost authority; fresh authorized lookup can recover it.
            if denied.is_some() {
                return Err(Error::Uncertain);
            }
            let old = self
                .volume
                .operation_by_id(grant.subject, receipt.retry.lineage, receipt.committed)
                .map_err(reply::error)?;
            super::query::receipt(old)?.part(p.op, p.context, 0)
        })();
        result.unwrap_or_else(|error| {
            let mut response = Packet::new(p.op);
            response.context = p.context;
            response.status = error as u8;
            response
        })
    }
}
