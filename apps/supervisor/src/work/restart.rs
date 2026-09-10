// SPDX-License-Identifier: Apache-2.0
//! Retain the retiring PID and its device work before issuing a new incarnation.
use super::super::services::*;
use rustic_sdk::{files::Client, rpc::Rpc, runtime::abi as k};
pub(super) struct Restart {
    pub initialize: bool,
    retired: bool,
    mount: super::mount::Mount,
}
impl Restart {
    pub fn new(initialize: bool) -> Self {
        Self {
            initialize,
            retired: false,
            mount: super::mount::Mount::new(),
        }
    }
    pub fn poll(&mut self, state: &mut State) -> Result<Option<[u64; 8]>, u64> {
        if !self.retired {
            state.work.phase = 2;
            if state.files != 0 {
                let _ = call([k::KILL, state.files, 0, 0, 0, 0, 0, 0]);
                state.work.io = call([k::INFO, 0, 0, 0, 0, 0, 0, 0]).map_err(|_| 4u64)?[6];
                if state.work.io != 0 {
                    return Ok(None);
                }
                call([k::REAP, state.files, 0, 0, 0, 0, 0, 0]).map_err(|_| 4u64)?;
                state.takeover.ended(state.files);
                state.files = 0;
            }
            let old = core::mem::replace(&mut state.owner, Client::new(0, 0, 0));
            let _ = old.close();
            let old = core::mem::replace(&mut state.admin, Rpc::new(0, 0));
            let _ = old.endpoint.close();
            state.work.io = 0;
            self.retired = true;
            return Ok(None);
        }
        state.work.phase = self.mount.phase();
        self.mount.poll(state, self.initialize)
    }
    pub fn cancel(&self, state: &mut State) {
        if self.retired {
            self.mount.cleanup(state);
            if state.files != 0 {
                let _ = call([k::KILL, state.files, 0, 0, 0, 0, 0, 0]);
            }
        }
        // A failed/timed-out mount keeps its PID for the next explicit drain/retry.
    }
}
