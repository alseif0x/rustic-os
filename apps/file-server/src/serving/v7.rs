// SPDX-License-Identifier: Apache-2.0
//! Explicit V7 dispatch: bounded reads, profile-2 tracked replacement and
//! profile-2 staged admission only. No V5 mutation or admission path is
//! reachable. Requests are served one at a time; a chunk that fills a sector,
//! a commit and the owner's retention maintenance perform blocking disk I/O.
//! An admission acceptance, execution or cancellation polls its publication
//! over the pollable disk and holds the client's reply until it settles; the
//! owner may revoke or detach clients between polls ([`control`]).
//! Maintenance is reachable only from the administrative channel.
mod control;

use rustic_file_service::{CLIENTS7, GrantRequest7, Server7};
use rustic_sdk::{
    abi::{files, runtime as wire},
    ipc::{Endpoint, Message},
    runtime,
};

const ADMIN_SLOT: usize = CLIENTS7;
const WORKSPACES_ROOT: u32 = 4;
/// Word 5 bit 0: profile-2 tracked writes are served.
const TRACKED_WRITES: u64 = 1;
/// Word 5 bit 1: profile-2 staged admissions are served.
const ADMISSIONS: u64 = 1 << 1;
/// Ready report: status, profile, nodes, maximum file bytes, retained records,
/// feature bits.
const READY: [u64; 8] = [0, 2, 256, 524288, 8, TRACKED_WRITES | ADMISSIONS, 0, 0];

/// Queued replies: one per client slot, then the administrative channel's.
type Replies = [Option<Message>; CLIENTS7 + 1];

fn close_slot(server: &mut Server7<'_>, replies: &mut Replies, slot: usize) {
    if let Some(grant) = server.grant_at(slot) {
        let _ = Endpoint::from_bootstrap(grant.endpoint).close();
    }
    server.detach(slot);
    replies[slot] = None;
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
                        Ok(words) => admin_request(server, disk, &mut replies, words),
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
            if let Some(grant) = server.grant_at(slot)
                && grant.expires != 0
                && now >= grant.expires
            {
                close_slot(server, &mut replies, slot);
                continue;
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
                            if let Some(code) = owner.finish() {
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
                    if server
                        .grant_at(slot)
                        .is_some_and(|current| current.endpoint == grant.endpoint)
                    {
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
                if let Some(grant) = server.grant_at(slot) {
                    tokens[count] = grant.endpoint;
                    count += 1;
                }
            }
            let _ = runtime::wait_set(&tokens[..count], 100);
        }
    }
}

fn admin_request(
    server: &mut Server7<'_>,
    disk: &mut super::super::disk::Disk,
    replies: &mut Replies,
    words: [u64; 8],
) -> [u64; 8] {
    let mut result = [0; 8];
    let parsed = (|| {
        let slot = usize::try_from(words[1]).map_err(|_| files::Error::Invalid)?;
        match words[0] {
            command if command == u64::from(files::GRANT) => {
                if slot >= CLIENTS7 || words[2] == 0 || words[3] == 0 {
                    return Err(files::Error::Invalid);
                }
                let old = server.grant_at(slot);
                let scope = if words[4] == 0 {
                    WORKSPACES_ROOT
                } else {
                    u32::try_from(words[4]).map_err(|_| files::Error::Invalid)?
                };
                // The service validates the rights profile and subject.
                let request = GrantRequest7 {
                    peer: words[2],
                    endpoint: words[3],
                    scope,
                    rights: u8::try_from(words[5]).map_err(|_| files::Error::Invalid)?,
                    subject: words[7],
                    expires: words[6],
                };
                let grant = server.grant(slot, request)?;
                replies[slot] = None;
                if let Some(old) = old
                    && old.endpoint != grant.endpoint
                {
                    let _ = Endpoint::from_bootstrap(old.endpoint).close();
                }
                result[1] = u64::from(grant.context);
            }
            command if command == u64::from(files::REVOKE) => {
                if slot >= CLIENTS7 || words[2..].iter().any(|word| *word != 0) {
                    return Err(files::Error::Invalid);
                }
                server.revoke(slot)?;
                close_slot(server, replies, slot);
            }
            34 if words[1..].iter().all(|word| *word == 0) => {}
            // Owner retention maintenance: the service refuses it with `Busy`
            // while any transfer, stage or unresolved admission is open.
            command
                if command == u64::from(files::MAINTAIN_RETENTION)
                    && words[1..].iter().all(|word| *word == 0) =>
            {
                let done = server.maintain_retention(disk)?;
                result[1] = done.previous_epoch;
                result[2] = done.epoch;
                result[3] = u64::from(done.records);
                result[4] = u64::from(done.sectors);
            }
            35 if slot < CLIENTS7 && words[2..].iter().all(|word| *word == 0) => {
                close_slot(server, replies, slot);
            }
            _ => return Err(files::Error::Protocol),
        }
        Ok(())
    })();
    if let Err(error) = parsed {
        result[0] = error as u64;
    }
    result
}
