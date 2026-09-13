// SPDX-License-Identifier: Apache-2.0
//! Authenticated bootstrap and private supervisor relay.

use rustic_sdk::{
    abi::{runtime as k, supervisor as s},
    files::Client,
    ipc::{Endpoint, Message},
};

pub fn run(files: u64, control: u64, peer: u64) -> u64 {
    if files == 0 || control == 0 || peer == 0 {
        return 1;
    }
    let endpoint = Endpoint::from_bootstrap(control);
    if endpoint.wait().is_err() {
        return 1;
    }
    let Ok(message) = endpoint.receive() else {
        return 2;
    };
    if message.correlation() != 0 || message.sender() == 0 {
        return 3;
    }
    let Ok(words) = k::decode(message.payload()) else {
        return 3;
    };
    if words[0] != s::TASKS
        || words[1] == 0
        || words[2] != 0
        || words[3] == 0
        || words[4..].iter().any(|word| *word != 0)
    {
        return 3;
    }
    let scope = match u32::try_from(words[1]) {
        Ok(scope) if scope != 0 => scope,
        _ => return 3,
    };
    let generation = match u32::try_from(words[3]) {
        Ok(generation) if generation != 0 => generation,
        _ => return 3,
    };
    let owner = message.sender();
    let mut client = Client::new(files, peer, generation);
    let mut state = super::read::State::new();
    loop {
        if endpoint.wait().is_err() {
            return 4;
        }
        let Ok(message) = endpoint.receive() else {
            return 5;
        };
        if message.sender() != owner {
            return 6;
        }
        let response = match k::decode(message.payload())
            .ok()
            .and_then(rustic_tasks_contract::wire::decode_request)
        {
            Some(rustic_tasks_contract::wire::Request::List) => state.list(&mut client, scope),
            Some(rustic_tasks_contract::wire::Request::Next) => state.next(),
            Some(rustic_tasks_contract::wire::Request::Preview(edit)) => {
                state.preview(&mut client, scope, edit)
            }
            Some(rustic_tasks_contract::wire::Request::Bytes(offset)) => {
                state.candidate_bytes(offset)
            }
            None => rustic_tasks_contract::wire::service(4),
        };
        let Ok(reply) = Message::new(message.correlation(), &k::encode(response)) else {
            return 7;
        };
        if endpoint.send(&reply).is_err() {
            return 8;
        }
    }
}
