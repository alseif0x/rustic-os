// SPDX-License-Identifier: Apache-2.0
//! Deterministic, owner-stepped native client. No authority is accepted from file data.
use rustic_sdk::{
    abi::{files as f, runtime as k, supervisor::actor as a},
    files::Client,
    ipc::{Endpoint, Message},
    runtime,
};

pub fn run(files: &mut Client, control: &Endpoint, owner: u64, scope: u32, other: u32) -> u64 {
    let mut version = 0;
    let mut sequence = 0;
    let mut read = super::read::State::default();
    loop {
        if control.wait().is_err() {
            return 1;
        }
        let Ok(message) = control.receive() else {
            return 2;
        };
        if message.sender() != owner {
            return 3;
        }
        let Ok(w) = k::decode(message.payload()) else {
            return 4;
        };
        let r = match w[0] {
            a::API_READ | a::READ_OPEN | a::READ_NEXT | a::FILL => {
                read.execute(w[0], files, scope, other)
            }
            a::READ => {
                let mut bytes = [0; 1024];
                let first = files.read(scope, &mut bytes);
                let inaccessible = files.stat(other).err().map_or(0, |e| e as u64);
                let denied = runtime::control([k::SPAWN, k::UTILITY, 0, 0, 0, 0, 0, 0])
                    == Err(runtime::Error::Denied)
                    && runtime::control([k::MOVE_ENDPOINT, 0, 0, 0, 3, 0, 0, 0])
                        == Err(runtime::Error::Denied)
                    && runtime::control([k::HOLD_COMPLETION, 0, 0, 400, 0, 0, 0, 0])
                        == Err(runtime::Error::Denied)
                    && runtime::control([k::OBSERVATION_STATUS, 0, 0, 0, 0, 0, 0, 0])
                        == Err(runtime::Error::Denied);
                if let Ok(meta) = files.stat(scope) {
                    version = meta.version;
                }
                [
                    first.as_ref().err().map_or(0, |e| *e as u64),
                    first.unwrap_or(0) as u64,
                    inaccessible,
                    denied as u64,
                    version,
                    0,
                    0,
                    0,
                ]
            }
            a::STAGE => {
                // Volatile staging only. COMMIT is separately owner-stepped so a human
                // edit or revocation can be placed between preparation and admission.
                let result = (|| {
                    let mut begin = f::Packet::new(f::BEGIN);
                    begin.id = scope;
                    begin.version = version;
                    begin.arg = 19;
                    files.request(begin)?;
                    let mut chunk = f::Packet::new(f::CHUNK);
                    chunk.id = scope;
                    chunk.count = 19;
                    chunk.data[..19].copy_from_slice(b"session client edit");
                    files.request(chunk)?;
                    Ok::<(), f::Error>(())
                })();
                [result.err().map_or(0, |e| e as u64), 0, 0, 0, 0, 0, 0, 0]
            }
            a::COMMIT => {
                let mut p = f::Packet::new(f::COMMIT);
                p.id = scope;
                match files.request(p) {
                    Ok(p) => [0, p.version, 0, 0, 0, 0, 0, 0],
                    Err(e) => [e as u64, 0, 0, 0, 0, 0, 0, 0],
                }
            }
            a::FLOOD => super::pressure::fill(files, scope, &mut sequence),
            a::DRAIN => super::pressure::drain(files),
            a::MOVED => {
                let mut moved = Client::new(w[1], w[2], w[3] as u32);
                let error = moved.stat(w[4] as u32).err().map_or(0, |e| e as u64);
                // Keep the moved endpoint owned until process exit; subsequent session
                // fencing must not depend on its original holder still possessing it.
                [error, 0, 0, 0, 0, 0, 0, 0]
            }
            a::STALE => {
                let mut p = f::Packet::new(f::STAT);
                p.id = scope;
                p.context = files.context;
                let rejected = Endpoint::from_bootstrap(files.token())
                    .send(&Message::new(999, &p.encode()).unwrap())
                    .is_err();
                [0, rejected as u64, 0, 0, 0, 0, 0, 0]
            }
            _ => [1, 0, 0, 0, 0, 0, 0, 0],
        };
        if control
            .send(&Message::new(message.correlation(), &k::encode(r)).unwrap())
            .is_err()
        {
            return 5;
        }
    }
}
