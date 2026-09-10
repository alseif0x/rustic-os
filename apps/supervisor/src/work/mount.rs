// SPDX-License-Identifier: Apache-2.0
//! Fresh service incarnation: ready -> owner grant -> policy -> shell grant.
use super::super::services::*;
use rustic_sdk::{files::Client, ipc::Endpoint, process, rpc::Rpc, runtime::abi as k};
pub(super) struct Mount {
    admin: [u64; 2],
    owner: [u64; 2],
    shell: [u64; 2],
    phase: u8,
    sent: bool,
    policy: super::policy::Policy,
}
impl Mount {
    pub fn new() -> Self {
        Self {
            admin: [0; 2],
            owner: [0; 2],
            shell: [0; 2],
            phase: 0,
            sent: false,
            policy: super::policy::Policy::new(),
        }
    }
    pub fn phase(&self) -> u64 {
        3 + self.phase as u64
    }
    pub fn cleanup(&self, state: &State) {
        let me = process::id().unwrap_or(0);
        close(me, self.admin[0]);
        close(me, self.owner[0]);
        close(state.shell, self.shell[1]);
    }
    pub fn poll(&mut self, state: &mut State, initialize: bool) -> Result<Option<[u64; 8]>, u64> {
        let me = process::id().map_err(|_| 4u64)?;
        match self.phase {
            0 => {
                state.files = spawn(k::FILES).map_err(|_| 3u64)?;
                self.admin = connect(me, state.files).map_err(|_| 3u64)?;
                self.owner = connect(me, state.files).map_err(|_| 3u64)?;
                let sectors = call([k::DEVICE, 0, 0, 0, 0, 0, 0, 0]).map_err(|_| 4u64)?[0];
                let block = call([k::BLOCK_GRANT, state.files, 7, 0, sectors, 0, 0, 0])
                    .map_err(|_| 4u64)?[0];
                start(state.files, [block, self.admin[1], initialize as u64]).map_err(|_| 4u64)?;
                self.phase = 1;
            }
            1 => {
                let m = match Endpoint::from_bootstrap(self.admin[0]).receive() {
                    Ok(m) => m,
                    Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {
                        return Ok(None);
                    }
                    Err(_) => return Err(4),
                };
                let w = k::decode(m.payload()).map_err(|_| 4u64)?;
                if m.sender() != state.files
                    || m.correlation() != 0
                    || w != [0, 1, 32, 1024, 0, 0, 0, 0]
                {
                    return Err(4);
                }
                state.admin = Rpc::new(self.admin[0], state.files);
                self.phase = 2;
            }
            2 => {
                if let Some(generation) =
                    self.grant(state, [32, 1, me, self.owner[1], 0, 7, 0, 1])?
                {
                    state.owner = Client::new(self.owner[0], state.files, generation);
                    self.phase = 3;
                }
            }
            3 => {
                if let Some(policy) = self.policy.poll(&mut state.owner) {
                    state.policy = policy;
                    self.shell = connect(state.files, state.shell).map_err(|_| 3u64)?;
                    self.phase = 4;
                }
            }
            4 => {
                if let Some(generation) =
                    self.grant(state, [32, 0, state.shell, self.shell[0], 0, 7, 0, 1])?
                {
                    return Ok(Some([
                        0,
                        state.files,
                        self.shell[1],
                        generation as u64,
                        0,
                        0,
                        0,
                        0,
                    ]));
                }
            }
            _ => return Err(4),
        }
        Ok(None)
    }
    fn grant(&mut self, state: &mut State, w: [u64; 8]) -> Result<Option<u32>, u64> {
        if !self.sent {
            match state.admin.begin(&k::encode(w)) {
                Ok(()) => self.sent = true,
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {
                    return Ok(None);
                }
                Err(_) => return Err(4),
            }
            return Ok(None);
        }
        let Some(m) = state.admin.poll().map_err(|_| 4u64)? else {
            return Ok(None);
        };
        let r = k::decode(m.payload()).map_err(|_| 4u64)?;
        if r[0] != 0 || r[1] == 0 || r[2..].iter().any(|x| *x != 0) {
            return Err(4);
        }
        self.sent = false;
        u32::try_from(r[1]).map(Some).map_err(|_| 4)
    }
}
