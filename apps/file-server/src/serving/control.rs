// SPDX-License-Identifier: Apache-2.0
//! Private owner IPC during one logical replacement; settlement precedes revoke ACK.
mod public;
use rustic_file_service::{CLIENTS, Clients, ExecutionQueue, Server};
use rustic_sdk::{
    abi::{
        files::{Error, Packet},
        runtime as wire,
    },
    ipc::{Endpoint, Message},
    runtime,
};

pub(super) struct Owner<'a> {
    admin: &'a Endpoint,
    administrator: &'a mut u64,
    replies: &'a mut [Option<Message>; CLIENTS + 1],
    deferred: Option<(u64, [u64; 8])>,
    pub(super) lost: bool,
}
impl<'a> Owner<'a> {
    pub(super) fn new(
        admin: &'a Endpoint,
        administrator: &'a mut u64,
        replies: &'a mut [Option<Message>; CLIENTS + 1],
    ) -> Self {
        Self {
            admin,
            administrator,
            replies,
            deferred: None,
            lost: false,
        }
    }
    pub(super) fn commit(
        &mut self,
        server: &mut Server,
        disk: &mut super::super::disk::Disk,
        slot: usize,
        peer: u64,
        request: Packet,
    ) -> Packet {
        let result = if request.op == rustic_sdk::abi::files::admission::EXECUTE {
            server.admission_execute_active(
                disk,
                rustic_file_service::Caller {
                    slot,
                    peer,
                    context: request.context,
                },
                request,
                runtime::clock(),
                |clients, active| self.active_poll(clients, active, slot),
            )
        } else if rustic_sdk::abi::files::admission::controlled(request.op) {
            server.admission_with(
                disk,
                rustic_file_service::Caller {
                    slot,
                    peer,
                    context: request.context,
                },
                request,
                |clients, pending| self.poll(clients, pending),
            )
        } else {
            server.commit_with(disk, slot, peer, request, |clients, pending| {
                self.poll(clients, pending)
            })
        };
        self.finish(server);
        result
    }
    pub(super) fn run_scheduled(
        &mut self,
        server: &mut Server,
        disk: &mut super::super::disk::Disk,
        queue: &mut ExecutionQueue,
    ) -> bool {
        let ran = server
            .run_scheduled(disk, queue, runtime::clock(), |clients, active, queue| {
                self.scheduled_poll(clients, active, queue)
            })
            .is_some();
        self.finish(server);
        ran
    }
    fn finish(&mut self, server: &Server) {
        if let Some((correlation, mut words)) = self.deferred.take() {
            // Neither an ACK nor a lost client reply is a rollback claim. Report
            // live settlement only after the admitted command was drained.
            words[3] = server.volume.stat(1).is_err() as u64;
            words[4] = server.volume.sequence();
            self.replies[CLIENTS] = Some(Message::new(correlation, &wire::encode(words)).unwrap());
        }
    }
    fn poll(&mut self, clients: &mut Clients, pending: bool) -> u64 {
        if !self.lost && self.deferred.is_none() {
            if let Some(reply) = &self.replies[CLIENTS] {
                match self.admin.send(reply) {
                    Ok(()) => self.replies[CLIENTS] = None,
                    Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => (),
                    Err(_) => self.lost = true,
                }
            }
            if !self.lost && self.replies[CLIENTS].is_none() {
                match self.admin.receive() {
                    Ok(message) => self.receive(clients, message),
                    Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => (),
                    Err(_) => self.lost = true,
                }
            }
        }
        if self.lost {
            for slot in 0..CLIENTS {
                clients.detach(slot);
                self.replies[slot] = None;
            }
        } else if pending {
            // One tick bounds latency; WAIT_SET currently watches IPC, not block
            // completions. The kernel continues servicing the device meanwhile.
            let _ = runtime::wait_set(&[self.admin.token()], 1);
        }
        runtime::clock()
    }
    fn receive(&mut self, clients: &mut Clients, message: Message) {
        if *self.administrator == 0 {
            *self.administrator = message.sender();
        }
        if message.sender() != *self.administrator {
            self.lost = true;
            return;
        }
        let mut words = [0; 8];
        let mut defer = false;
        let result = (|| {
            let w = wire::decode(message.payload()).map_err(|_| Error::Invalid)?;
            let slot = usize::try_from(w[1]).map_err(|_| Error::Invalid)?;
            let before = clients.pending();
            match w[0] {
                33 if w[2..].iter().all(|x| *x == 0) => {
                    words[1] = clients.revoke(slot)? as u64;
                    defer = true;
                }
                40 if slot != 0 && w[2..].iter().all(|x| *x == 0) => {
                    words[1] = clients.revoke_root(u32::try_from(w[1]).map_err(|_| Error::Invalid)?)
                        as u64;
                    defer = true;
                }
                35 if slot < CLIENTS && w[2..].iter().all(|x| *x == 0) => {
                    if let Some(grant) = clients.grant_at(slot) {
                        let _ = Endpoint::from_bootstrap(grant.endpoint).close();
                    }
                    clients.detach(slot);
                    self.replies[slot] = None;
                }
                34 if w[1..].iter().all(|x| *x == 0) => words[1] = clients.pending() as u64,
                // New grants, storage administration and deliberate service stalls
                // need the ordinary dispatch boundary; they cannot run under I/O.
                _ => return Err(Error::Busy),
            }
            if defer {
                words[2] = (before - clients.pending()) as u64;
            }
            for (slot, reply) in self.replies[..CLIENTS].iter_mut().enumerate() {
                if clients.grant_at(slot).is_none_or(|grant| grant.rights == 0) {
                    *reply = None;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            words[0] = error as u64;
        }
        if defer {
            self.deferred = Some((message.correlation(), words));
        } else {
            self.replies[CLIENTS] =
                Some(Message::new(message.correlation(), &wire::encode(words)).unwrap());
        }
    }
}
