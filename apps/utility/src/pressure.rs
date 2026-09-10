// SPDX-License-Identifier: Apache-2.0
//! Bounded queue-pressure fixture; bulk file replies are intentionally left unread.
use rustic_sdk::{
    abi::{files as f, ipc as i},
    files::Client,
    ipc::{Endpoint, Message},
    runtime,
};
pub(super) fn fill(files: &Client, id: u32, sequence: &mut u64) -> [u64; 8] {
    let endpoint = Endpoint::from_bootstrap(files.token());
    let mut sent = 0;
    let mut blocked = 0;
    let deadline = runtime::clock().saturating_add(20);
    while runtime::clock() < deadline {
        let mut p = f::Packet::new(f::STAT);
        p.id = id;
        p.context = files.context;
        *sequence += 1;
        match endpoint.send(&Message::new(10000 + *sequence, &p.encode()).unwrap()) {
            Ok(()) => sent += 1,
            Err(rustic_sdk::Error::Ipc(i::Error::WouldBlock)) => blocked += 1,
            Err(_) => return [1, sent, blocked, 0, 0, 0, 0, 0],
        }
    }
    [0, sent, blocked, 0, 0, 0, 0, 0]
}
pub(super) fn drain(files: &Client) -> [u64; 8] {
    let endpoint = Endpoint::from_bootstrap(files.token());
    let mut total = 0;
    let mut revoked = 0;
    let deadline = runtime::clock().saturating_add(100);
    while runtime::clock() < deadline {
        match endpoint.receive() {
            Ok(message) => {
                let Ok(p) = f::Packet::decode(message.payload()) else {
                    return [1, 0, 0, 0, 0, 0, 0, 0];
                };
                if message.correlation() < 10000 || p.context != files.context {
                    return [1, 0, 0, 0, 0, 0, 0, 0];
                }
                total += 1;
                if p.status == f::Error::Revoked as u8 {
                    revoked += 1;
                }
            }
            Err(rustic_sdk::Error::Ipc(i::Error::WouldBlock)) => {
                let _ = runtime::wait_set(&[endpoint.token()], 1);
            }
            Err(_) => return [1, total, revoked, 0, 0, 0, 0, 0],
        }
    }
    [0, total, revoked, 0, 0, 0, 0, 0]
}
