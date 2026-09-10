// SPDX-License-Identifier: Apache-2.0
use super::children::Child;
use rustic_sdk::{
    files::Client,
    ipc::Endpoint,
    rpc::Rpc,
    runtime::{self, abi as k},
};
pub struct State {
    pub files: u64,
    pub shell: u64,
    pub admin: Rpc,
    pub owner: Client,
    pub control: Endpoint,
    pub children: [Option<Child>; 2],
    pub policy: u32,
    pub(super) takeover: super::takeover::Takeover,
    pub(super) degraded: bool,
    pub(super) stopping: bool,
}
pub fn call(w: [u64; 8]) -> Result<[u64; 8], ()> {
    runtime::control(w).map_err(|_| ())
}
pub fn spawn(program: u64) -> Result<u64, ()> {
    Ok(call([k::SPAWN, program, 0, 0, 0, 0, 0, 0])?[0])
}
pub fn connect(a: u64, b: u64) -> Result<[u64; 2], ()> {
    let r = call([k::CONNECT, a, b, 0, 0, 0, 0, 0])?;
    Ok([r[0], r[1]])
}
pub fn start(pid: u64, args: [u64; 3]) -> Result<(), ()> {
    call([k::START, pid, args[0], args[1], args[2], 0, 0, 0])?;
    Ok(())
}
pub fn grant(
    admin: &mut Rpc,
    slot: usize,
    peer: u64,
    endpoint: u64,
    scope: u32,
    rights: u8,
    expires: u64,
) -> Result<u32, ()> {
    let r = admin
        .words([
            32,
            slot as u64,
            peer,
            endpoint,
            scope as u64,
            rights as u64,
            expires,
            0,
        ])
        .map_err(|_| ())?;
    if r[0] != 0 {
        return Err(());
    }
    u32::try_from(r[1]).map_err(|_| ())
}
/// The local owner is a stable recovery subject, never a client-supplied identity.
pub fn owner_grant(admin: &mut Rpc, slot: usize, peer: u64, endpoint: u64) -> Result<u32, ()> {
    let r = admin
        .words([32, slot as u64, peer, endpoint, 0, 7, 0, 1])
        .map_err(|_| ())?;
    if r[0] != 0 {
        return Err(());
    }
    u32::try_from(r[1]).map_err(|_| ())
}
pub fn detach(admin: &mut Rpc, slot: usize) -> Result<(), ()> {
    let r = admin
        .words([35, slot as u64, 0, 0, 0, 0, 0, 0])
        .map_err(|_| ())?;
    if r[0] == 0 { Ok(()) } else { Err(()) }
}
pub fn close(owner: u64, token: u64) {
    if token != 0 {
        let _ = call([k::CLOSE_ENDPOINT, owner, token, 0, 0, 0, 0, 0]);
    }
}
pub fn stop(pid: u64) {
    let _ = call([k::KILL, pid, 0, 0, 0, 0, 0, 0]);
    let _ = call([k::REAP, pid, 0, 0, 0, 0, 0, 0]);
}
impl State {
    pub fn serve(&mut self) -> u64 {
        loop {
            self.collect();
            self.poll_takeover();
            match self.control.receive() {
                Ok(message) => {
                    if message.sender() != self.shell {
                        return 2;
                    }
                    let result = match k::decode(message.payload()) {
                        Ok(w) => self.request(w),
                        Err(_) => Err(1),
                    };
                    let words = match result {
                        Ok(w) => w,
                        Err(code) => [code, 0, 0, 0, 0, 0, 0, 0],
                    };
                    let response =
                        rustic_sdk::ipc::Message::new(message.correlation(), &k::encode(words))
                            .unwrap();
                    if self.control.send(&response).is_err() {
                        return 3;
                    }
                }
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {
                    let mut tokens = [0; 4];
                    tokens[0] = self.control.token();
                    let mut n = 1;
                    for c in self.children.iter().flatten() {
                        if !c.closed {
                            tokens[n] = c.control.endpoint.token();
                            n += 1;
                        }
                    }
                    if self.admin.pending() && !self.admin.failed() {
                        tokens[n] = self.admin.endpoint.token();
                        n += 1;
                    }
                    let timeout = if self.takeover.pending()
                        || self.children.iter().flatten().any(|c| c.control.pending())
                    {
                        1
                    } else {
                        100
                    };
                    let _ = runtime::wait_set(&tokens[..n], timeout);
                }
                Err(_) => {
                    let _ = call([k::SHUTDOWN, 0, 0, 0, 0, 0, 0, 0]);
                    return 0;
                }
            }
        }
    }
}
