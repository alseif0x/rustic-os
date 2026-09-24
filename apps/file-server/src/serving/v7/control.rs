// SPDX-License-Identifier: Apache-2.0
//! Private owner IPC and client progress between the polls of one V7
//! admission publication; its settlement precedes a revocation's ACK.
//!
//! The service calls [`Owner::poll`] before every poll of the publication.
//! It first gives the administrative channel its opportunity (a revocation
//! applies at once and is acknowledged after settlement; see
//! [`rustic_file_server::publication`]), then keeps the other clients'
//! transport moving: queued replies are retried and new requests are answered
//! `Busy`. The publishing client's endpoint is not read: its reply is
//! outstanding. Device completion is not a `WAIT_SET` source, so a command
//! still pending at a second consecutive opportunity costs one tick.
use super::{ADMIN_SLOT, Replies};
use rustic_file_server::publication::{self, Admin7};
use rustic_file_service::{CLIENTS7, Control7};
use rustic_sdk::{
    abi::{files, runtime as wire},
    ipc::{Endpoint, Message},
    runtime,
};

pub(super) struct Owner<'a> {
    admin: &'a Endpoint,
    administrator: &'a mut u64,
    replies: &'a mut Replies,
    publishing: usize,
    /// Correlation of a revocation acknowledged after settlement.
    deferred: Option<u64>,
    /// The previous opportunity already saw the command outstanding.
    waiting: bool,
    /// The serving loop's exit code once the administrative channel is lost.
    exit: Option<u64>,
}

impl<'a> Owner<'a> {
    pub(super) fn new(
        admin: &'a Endpoint,
        administrator: &'a mut u64,
        replies: &'a mut Replies,
        publishing: usize,
    ) -> Self {
        Self {
            admin,
            administrator,
            replies,
            publishing,
            deferred: None,
            waiting: false,
            exit: None,
        }
    }

    /// One owner-control opportunity; returns the owner's clock.
    pub(super) fn poll(&mut self, control: &mut Control7<'_>) -> u64 {
        if self.exit.is_none() && self.deferred.is_none() {
            self.administer(control);
        }
        if self.exit.is_some() {
            // Without its owner the service can grant nothing: every client
            // loses its authority, which also stops an unsettled publication.
            for slot in 0..CLIENTS7 {
                self.close(control, slot);
            }
            return runtime::clock();
        }
        self.clients(control);
        let pending = control.pending();
        if pending && self.waiting {
            let _ = runtime::wait_set(&[self.admin.token()], 1);
        }
        self.waiting = pending;
        runtime::clock()
    }

    /// Queue the deferred revocation ACK once the request has settled, and
    /// return the serving loop's exit code if the owner was lost.
    pub(super) fn finish(self) -> Option<u64> {
        if let Some(correlation) = self.deferred {
            self.replies[ADMIN_SLOT] =
                Some(Message::new(correlation, &wire::encode([0; 8])).unwrap());
        }
        self.exit
    }

    fn administer(&mut self, control: &mut Control7<'_>) {
        if let Some(reply) = &self.replies[ADMIN_SLOT] {
            match self.admin.send(reply) {
                Ok(()) => self.replies[ADMIN_SLOT] = None,
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => return,
                Err(_) => {
                    self.exit = Some(2);
                    return;
                }
            }
        }
        let message = match self.admin.receive() {
            Ok(message) => message,
            Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => return,
            Err(_) => {
                self.exit = Some(0);
                return;
            }
        };
        if *self.administrator == 0 {
            *self.administrator = message.sender();
        }
        if message.sender() != *self.administrator {
            self.exit = Some(3);
            return;
        }
        let mut output = [0; 8];
        match wire::decode(message.payload()) {
            Err(_) => output[0] = files::Error::Invalid as u64,
            Ok(words) => match publication::admin(words, CLIENTS7) {
                Admin7::Revoke(slot) => match control.revoke(slot) {
                    Ok(()) => {
                        self.close(control, slot);
                        self.deferred = Some(message.correlation());
                        return;
                    }
                    Err(error) => output[0] = error as u64,
                },
                Admin7::Detach(slot) => self.close(control, slot),
                Admin7::Probe => {}
                Admin7::Refuse(error) => output[0] = error as u64,
            },
        }
        self.replies[ADMIN_SLOT] =
            Some(Message::new(message.correlation(), &wire::encode(output)).unwrap());
    }

    fn clients(&mut self, control: &mut Control7<'_>) {
        let now = runtime::clock();
        for slot in 0..CLIENTS7 {
            if slot == self.publishing {
                continue;
            }
            let Some(grant) = control.grant_at(slot) else {
                self.replies[slot] = None;
                continue;
            };
            if grant.expires != 0 && now >= grant.expires {
                self.close(control, slot);
                continue;
            }
            let endpoint = Endpoint::from_bootstrap(grant.endpoint);
            if let Some(reply) = &self.replies[slot] {
                match endpoint.send(reply) {
                    Ok(()) => self.replies[slot] = None,
                    Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {
                        continue;
                    }
                    Err(_) => {
                        self.close(control, slot);
                        continue;
                    }
                }
            }
            match endpoint.receive() {
                Ok(message) => {
                    let request = files::Packet::decode(message.payload()).ok();
                    let reply = publication::busy(request.as_ref());
                    self.replies[slot] =
                        Some(Message::new(message.correlation(), &reply.encode()).unwrap());
                }
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {}
                Err(_) => self.close(control, slot),
            }
        }
    }

    /// Close the slot's endpoint and forget the slot and its queued reply.
    fn close(&mut self, control: &mut Control7<'_>, slot: usize) {
        if let Some(grant) = control.grant_at(slot) {
            let _ = Endpoint::from_bootstrap(grant.endpoint).close();
        }
        control.detach(slot);
        self.replies[slot] = None;
    }
}
