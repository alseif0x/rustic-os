// SPDX-License-Identifier: Apache-2.0
//! Fair bounded dispatch: one request per client per pass, owned pending replies.
mod control;
use rustic_file_service::{CLIENTS, Caller, ExecutionQueue, Server};
use rustic_sdk::{
    abi::{
        files::{Packet, admission},
        runtime as wire,
    },
    ipc::{Endpoint, Message},
    runtime,
};
fn detach(server: &mut Server, slot: usize) {
    if let Some(g) = server.grant_at(slot) {
        let _ = Endpoint::from_bootstrap(g.endpoint).close();
    }
    server.detach(slot);
}
// Keep startup-owned buffers out of the live request stack.
#[inline(never)]
pub fn run(disk: &mut super::disk::Disk, server: &mut Server, admin: Endpoint) -> u64 {
    let ready = Message::new(0, &wire::encode([0, 1, 32, 1024, 0, 0, 0, 0])).unwrap();
    if admin.send(&ready).is_err() {
        return 1;
    }
    let mut administrator = 0;
    let mut replies: [Option<Message>; CLIENTS + 1] = [const { None }; CLIENTS + 1];
    let mut queue = ExecutionQueue::new();
    loop {
        // Replies are retried without stopping progress of unrelated clients.
        for (slot, reply) in replies.iter_mut().enumerate() {
            if slot < CLIENTS && reply.is_some() {
                match server.grant_at(slot) {
                    Some(grant) => control::replies::restrict(
                        reply,
                        control::replies::denial(&grant, runtime::clock()),
                    ),
                    None => *reply = None,
                }
            }
            if let Some(message) = reply {
                let token = if slot == CLIENTS {
                    admin.token()
                } else {
                    server.grant_at(slot).map_or(0, |g| g.endpoint)
                };
                let endpoint = Endpoint::from_bootstrap(token);
                match endpoint.send(message) {
                    Ok(()) => *reply = None,
                    Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {}
                    Err(_) => {
                        *reply = None;
                        if slot == CLIENTS {
                            return 2;
                        }
                        detach(server, slot);
                    }
                }
            }
        }
        if replies[CLIENTS].is_none() {
            match admin.receive() {
                Ok(message) => {
                    if administrator == 0 {
                        administrator = message.sender();
                    }
                    if message.sender() != administrator {
                        return 3;
                    }
                    let output = match wire::decode(message.payload()) {
                        Ok(words) => {
                            if super::diagnostics::stall(&admin, message.correlation(), words) {
                                continue;
                            }
                            let slot = words[1] as usize;
                            let old = server.grant_at(slot);
                            let r = super::admin::dispatch(server, disk, words, runtime::clock());
                            if r[0] == 0 && matches!(words[0], 32 | 33 | 35 | 37) && slot < CLIENTS
                            {
                                // Never deliver a queued reply into a newly granted context.
                                replies[slot] = None;
                                if let Some(old) = old
                                    && server
                                        .grant_at(slot)
                                        .is_none_or(|g| g.endpoint != old.endpoint)
                                {
                                    let _ = Endpoint::from_bootstrap(old.endpoint).close();
                                }
                            }
                            if r[0] == 0 && matches!(words[0], 33 | 40) {
                                for (index, reply) in replies[..CLIENTS].iter_mut().enumerate() {
                                    if r[1] & (1 << index) != 0 {
                                        *reply = None;
                                    }
                                }
                            }
                            r
                        }
                        Err(_) => [1, 0, 0, 0, 0, 0, 0, 0],
                    };
                    replies[CLIENTS] =
                        Some(Message::new(message.correlation(), &wire::encode(output)).unwrap());
                }
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {}
                Err(_) => return 0,
            }
        }
        let now = runtime::clock();
        server.expire(now);
        for slot in 0..CLIENTS {
            if replies[slot].is_some() {
                continue;
            }
            let Some(grant) = server.grant_at(slot) else {
                continue;
            };
            let endpoint = Endpoint::from_bootstrap(grant.endpoint);
            match endpoint.receive() {
                Ok(message) => {
                    let output = match Packet::decode(message.payload()) {
                        Ok(request) => {
                            if request.op == admission::SCHEDULE
                                || !queue.is_empty() && admission::live(request.op)
                            {
                                server.scheduling_request(
                                    &mut queue,
                                    Caller {
                                        slot,
                                        peer: message.sender(),
                                        context: request.context,
                                    },
                                    request,
                                    runtime::clock(),
                                )
                            } else if request.op == rustic_sdk::abi::files::REPLACE_COMMIT
                                || rustic_sdk::abi::files::admission::controlled(request.op)
                            {
                                let mut owner =
                                    control::Owner::new(&admin, &mut administrator, &mut replies);
                                let output =
                                    owner.commit(server, disk, slot, message.sender(), request);
                                if owner.lost {
                                    return 0;
                                }
                                output
                            } else {
                                server.handle(
                                    disk,
                                    slot,
                                    message.sender(),
                                    request,
                                    runtime::clock(),
                                )
                            }
                        }
                        Err(_) => {
                            let mut p = Packet::new(1);
                            p.status = 1;
                            p
                        }
                    };
                    if server
                        .grant_at(slot)
                        .is_some_and(|current| current.endpoint == grant.endpoint)
                    {
                        replies[slot] =
                            Some(Message::new(message.correlation(), &output.encode()).unwrap());
                    }
                }
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {}
                Err(_) => detach(server, slot),
            }
        }
        // The scheduling reply is owned independently from execution. Its client
        // participates in active dispatch even if that reply is unread or lost.
        let mut owner = control::Owner::new(&admin, &mut administrator, &mut replies);
        let ran = owner.run_scheduled(server, disk, &mut queue);
        if owner.lost {
            return 0;
        }
        if ran {
            continue;
        }
        let mut tokens = [0; CLIENTS + 1];
        tokens[0] = admin.token();
        let mut n = 1;
        for slot in 0..CLIENTS {
            if let Some(grant) = server.grant_at(slot) {
                tokens[n] = grant.endpoint;
                n += 1;
            }
        }
        // Pending output retries on the next scheduler round; idle waits include admin.
        if replies.iter().all(Option::is_none) {
            let _ = runtime::wait_set(&tokens[..n], 100);
        }
    }
}
