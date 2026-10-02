// SPDX-License-Identifier: Apache-2.0
//! Explicit V7 dispatch: namespace operations, bounded reads and ordinary
//! replacement, profile-1/profile-2 tracked replacement and staged admission.
//! Requests are served one at a time; ordinary publication, a streamed chunk
//! that fills a sector and retention maintenance use blocking I/O.
//! A tracked commit or admission acceptance, execution or cancellation polls its publication
//! over the pollable disk and holds the client's reply until it settles; the
//! owner may revoke or detach clients between polls ([`control`]).
//! Maintenance is reachable only from the administrative channel.
mod admin;
mod control;

use rustic_file_service::{CLIENTS7, Server7};
use rustic_sdk::{
    abi::{files, runtime as wire},
    ipc::{Endpoint, Message},
    runtime,
};

const ADMIN_SLOT: usize = CLIENTS7;
/// Word 5 bit 0: tracked writes are served through both existing receipt profiles.
const TRACKED_WRITES: u64 = 1;
/// Word 5 bit 1: staged admissions accept both existing OPEN profiles.
const ADMISSIONS: u64 = 1 << 1;
/// Ready report: status, profile, nodes, maximum file bytes, retained records,
/// feature bits.
const READY: [u64; 8] = [0, 2, 256, 524288, 8, TRACKED_WRITES | ADMISSIONS, 0, 0];

/// Queued replies: one per client slot, then the administrative channel's.
type Replies = [Option<Message>; CLIENTS7 + 1];

fn close_slot(server: &mut Server7<'_>, replies: &mut Replies, slot: usize) {
    admin::detach_slot(server, replies, slot);
}

pub(crate) fn run(
    disk: &mut super::super::disk::Disk,
    server: &mut Server7<'_>,
    admin: Endpoint,
) -> u64 {
    let ready = Message::new(0, &wire::encode(READY)).unwrap();
    if admin.send(&ready).is_err() {
        return 1;
    }

    let mut administrator = 0;
    let mut replies: Replies = [const { None }; CLIENTS7 + 1];
    loop {
        for slot in 0..=ADMIN_SLOT {
            if slot < CLIENTS7 && replies[slot].is_some() {
                let now = runtime::clock();
                match server.grant_at(slot) {
                    Some(grant) if grant.rights == 0 => {
                        admin::discard_revoked_slot(server, &mut replies, slot)
                    }
                    Some(grant) if grant.expires == 0 || now < grant.expires => {}
                    Some(_) => close_slot(server, &mut replies, slot),
                    None => replies[slot] = None,
                }
            }
            if let Some(message) = &replies[slot] {
                let token = if slot == ADMIN_SLOT {
                    admin.token()
                } else {
                    server.grant_at(slot).map_or(0, |grant| grant.endpoint)
                };
                match Endpoint::from_bootstrap(token).send(message) {
                    Ok(()) => replies[slot] = None,
                    Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {}
                    Err(_) if slot == ADMIN_SLOT => return 2,
                    Err(_) => close_slot(server, &mut replies, slot),
                }
            }
        }

        if replies[ADMIN_SLOT].is_none() {
            match admin.receive() {
                Ok(message) => {
                    if administrator == 0 {
                        administrator = message.sender();
                    }
                    if message.sender() != administrator {
                        return 3;
                    }
                    let output = match wire::decode(message.payload()) {
                        Ok(words) => admin::request(server, disk, &mut replies, words),
                        Err(_) => [files::Error::Invalid as u64, 0, 0, 0, 0, 0, 0, 0],
                    };
                    replies[ADMIN_SLOT] =
                        Some(Message::new(message.correlation(), &wire::encode(output)).unwrap());
                }
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {}
                Err(_) => return 0,
            }
        }

        let now = runtime::clock();
        for slot in 0..CLIENTS7 {
            if let Some(grant) = server.grant_at(slot) {
                if grant.rights == 0 {
                    admin::discard_revoked_slot(server, &mut replies, slot);
                    continue;
                }
                if grant.expires != 0 && now >= grant.expires {
                    close_slot(server, &mut replies, slot);
                    continue;
                }
            }
            if replies[slot].is_some() {
                continue;
            }
            let Some(grant) = server.grant_at(slot) else {
                continue;
            };
            match Endpoint::from_bootstrap(grant.endpoint).receive() {
                Ok(message) => {
                    let response = match files::Packet::decode(message.payload()) {
                        Ok(request) => {
                            let mut owner =
                                control::Owner::new(&admin, &mut administrator, &mut replies, slot);
                            let response = server.handle_with(
                                disk,
                                slot,
                                message.sender(),
                                request,
                                runtime::clock(),
                                |control| owner.poll(control),
                            );
                            if let Some(code) = owner.finish(server) {
                                return code;
                            }
                            response
                        }
                        Err(_) => {
                            let mut response = files::Packet::new(1);
                            response.status = files::Error::Protocol as u8;
                            response
                        }
                    };
                    if server.grant_at(slot).is_some_and(|current| {
                        current.rights != 0
                            && current.context == grant.context
                            && current.endpoint == grant.endpoint
                    }) {
                        replies[slot] =
                            Some(Message::new(message.correlation(), &response.encode()).unwrap());
                    }
                }
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {}
                Err(_) => close_slot(server, &mut replies, slot),
            }
        }

        if replies.iter().all(Option::is_none) {
            let mut tokens = [0; CLIENTS7 + 1];
            tokens[0] = admin.token();
            let mut count = 1;
            for slot in 0..CLIENTS7 {
                if let Some(grant) = server.grant_at(slot)
                    && grant.rights != 0
                {
                    tokens[count] = grant.endpoint;
                    count += 1;
                }
            }
            let _ = runtime::wait_set(&tokens[..count], 100);
        }
    }
}
