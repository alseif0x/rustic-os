// SPDX-License-Identifier: Apache-2.0
//! Bounded transport progress, separate from the service's live authority view.
use super::Owner;
use rustic_file_service::{ActiveExecution, CLIENTS, Caller, Clients};
use rustic_sdk::{
    abi::files::{Error, Packet, admission},
    ipc::{Endpoint, Message},
    runtime,
};

impl Owner<'_> {
    pub(super) fn active_poll(
        &mut self,
        clients: &mut Clients,
        active: &mut ActiveExecution,
        executing: usize,
    ) -> u64 {
        // Private control always gets the first opportunity. It may fence clients
        // and discard their queued replies before any public response is sent.
        self.poll(clients, false);
        for slot in 0..CLIENTS {
            if slot == executing {
                continue;
            }
            let Some(grant) = clients.grant_at(slot) else {
                self.replies[slot] = None;
                continue;
            };
            let endpoint = Endpoint::from_bootstrap(grant.endpoint);
            if grant.rights == 0 || grant.expires != 0 && runtime::clock() >= grant.expires {
                self.replies[slot] = None;
                continue;
            }
            if let Some(reply) = &self.replies[slot] {
                match endpoint.send(reply) {
                    Ok(()) => self.replies[slot] = None,
                    Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {
                        continue;
                    }
                    Err(_) => {
                        clients.detach(slot);
                        self.replies[slot] = None;
                        let _ = endpoint.close();
                        continue;
                    }
                }
            }
            match endpoint.receive() {
                Ok(message) => {
                    let output = match Packet::decode(message.payload()) {
                        Ok(p) if admission::live(p.op) => active.request(
                            clients,
                            Caller {
                                slot,
                                peer: message.sender(),
                                context: p.context,
                            },
                            p,
                            runtime::clock(),
                        ),
                        Ok(p) => {
                            // Ordinary storage calls cannot reenter the borrowed
                            // publication. No transfer is consumed or replaced.
                            let mut r = Packet::new(p.op);
                            r.context = p.context;
                            r.status = Error::Busy as u8;
                            r
                        }
                        Err(_) => {
                            let mut r = Packet::new(1);
                            r.status = Error::Protocol as u8;
                            r
                        }
                    };
                    self.replies[slot] =
                        Some(Message::new(message.correlation(), &output.encode()).unwrap());
                }
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => (),
                Err(_) => {
                    clients.detach(slot);
                    let _ = endpoint.close();
                }
            }
        }
        if active.pending() {
            // Device completion is not a WAIT_SET source. One tick bounds polling
            // latency; pending reply retries cannot monopolize the service.
            let _ = runtime::wait_set(&[self.admin.token()], 1);
        }
        runtime::clock()
    }
}
