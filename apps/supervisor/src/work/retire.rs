// SPDX-License-Identifier: Apache-2.0
//! Drain and reap the previous file-service incarnation before mounting another.
use super::super::services::*;
use rustic_sdk::{files::Client, rpc::Rpc, runtime::abi as k};

pub(super) struct Retire {
    done: bool,
}

impl Retire {
    pub fn new() -> Self {
        Self { done: false }
    }

    pub fn poll(&mut self, state: &mut State) -> Result<bool, u64> {
        if self.done {
            return Ok(true);
        }
        state.work.phase = 2;
        if state.deferred_adopted != 0 {
            let pid = state.deferred_adopted;
            if !retire_pid(state, pid)? {
                return Ok(false);
            }
            state.deferred_adopted = 0;
        }
        if state.files != 0 {
            let pid = state.files;
            if !retire_pid(state, pid)? {
                return Ok(false);
            }
            state.takeover.ended(pid);
            state.files = 0;
        }
        let old = core::mem::replace(&mut state.owner, Client::new(0, 0, 0));
        let _ = old.close();
        let old = core::mem::replace(&mut state.admin, Rpc::new(0, 0));
        let _ = old.endpoint.close();
        state.work.io = 0;
        self.done = true;
        Ok(true)
    }
}

/// Kill, wait for outstanding device work, and reap one supervisor-owned child.
/// A false result means the caller should poll again after the pending I/O drains.
pub(super) fn retire_pid(state: &mut State, pid: u64) -> Result<bool, u64> {
    let _ = call([k::KILL, pid, 0, 0, 0, 0, 0, 0]);
    state.work.io = call([k::INFO, 0, 0, 0, 0, 0, 0, 0]).map_err(|_| 4u64)?[6];
    if state.work.io != 0 {
        return Ok(false);
    }
    call([k::REAP, pid, 0, 0, 0, 0, 0, 0]).map_err(|_| 4u64)?;
    Ok(true)
}
