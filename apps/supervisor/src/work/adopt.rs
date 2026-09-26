// SPDX-License-Identifier: Apache-2.0
//! Adopt one staged V7 file-server image as the running service.
use super::super::services::*;
use super::Task;
use rustic_sdk::{abi::supervisor as s, files::Client, rpc::Rpc};
use rustic_supervisor::storage_launch::plan_file_service_adoption;

#[derive(Clone, Copy)]
enum Phase {
    Retiring,
    Mounting,
    Cleaning { status: u64, endpoints_closed: bool },
}

pub(super) struct Adopt {
    pid: u64,
    retired: bool,
    retire: super::retire::Retire,
    mount: super::mount::Mount,
    phase: Phase,
}

impl Adopt {
    fn new(pid: u64) -> Self {
        Self {
            pid,
            retired: false,
            retire: super::retire::Retire::new(),
            mount: super::mount::Mount::adopted(pid),
            phase: Phase::Retiring,
        }
    }

    pub fn poll(&mut self, state: &mut State) -> Result<Option<[u64; 8]>, u64> {
        match self.phase {
            Phase::Retiring => match self.retire.poll(state) {
                Ok(true) => {
                    self.retired = true;
                    self.phase = Phase::Mounting;
                    Ok(None)
                }
                Ok(false) => Ok(None),
                Err(status) => {
                    self.fail(status, state);
                    Ok(None)
                }
            },
            Phase::Mounting => {
                state.work.phase = self.mount.phase();
                match self.mount.poll(state, false, FileProfile::V7) {
                    Ok(Some(words)) => Ok(Some(words)),
                    Ok(None) => Ok(None),
                    Err(status) => {
                        self.fail(status, state);
                        Ok(None)
                    }
                }
            }
            Phase::Cleaning {
                status,
                endpoints_closed,
            } => self.cleanup(state, status, endpoints_closed),
        }
    }

    /// Convert an expired startup into cleanup work. Cleanup is allowed to
    /// outlive the mount deadline so the adopted process is never abandoned.
    pub fn timeout(&mut self, state: &mut State) {
        self.fail(4, state);
    }

    fn fail(&mut self, status: u64, state: &mut State) {
        state.degraded = true;
        state.stopping = true;
        self.phase = Phase::Cleaning {
            status,
            endpoints_closed: false,
        };
    }

    fn cleanup(
        &mut self,
        state: &mut State,
        status: u64,
        endpoints_closed: bool,
    ) -> Result<Option<[u64; 8]>, u64> {
        if !endpoints_closed {
            self.mount.cleanup(state);
            if self.retired || state.files == self.pid {
                state.owner = Client::new(0, 0, 0);
                state.admin = Rpc::new(0, 0);
                state.admin_drain = false;
            }
            let _ = call([rustic_sdk::runtime::abi::KILL, self.pid, 0, 0, 0, 0, 0, 0]);
            self.phase = Phase::Cleaning {
                status,
                endpoints_closed: true,
            };
            return Ok(None);
        }

        if !matches!(super::retire::retire_pid(state, self.pid), Ok(true)) {
            return Ok(None);
        }
        if state.files == self.pid {
            state.takeover.ended(self.pid);
            state.files = 0;
        }
        state.work.io = 0;
        state.degraded = true;
        state.stopping = true;
        Ok(Some([status, 0, 0, 0, 0, 0, 0, 0]))
    }

    /// When a later `restart files` supersedes this job, transfer a candidate
    /// not yet installed as `state.files` to restart's shared drain/reap path.
    pub fn cancel(&mut self, state: &mut State) {
        self.mount.cleanup(state);
        let _ = call([rustic_sdk::runtime::abi::KILL, self.pid, 0, 0, 0, 0, 0, 0]);
        if state.files != self.pid {
            state.deferred_adopted = self.pid;
        }
        state.degraded = true;
        state.stopping = true;
    }
}

impl State {
    /// Owner request [`ADOPT_FILES_V7`](s::ADOPT_FILES_V7).
    pub(in super::super) fn adopt_files_v7(&mut self, pid: u64) -> Result<[u64; 8], u64> {
        if self.profile != FileProfile::V7 {
            return Err(1);
        }
        if self.stopping || !self.work.can_start() {
            return Err(3);
        }
        let staged = self.staged.filter(|staged| staged.pid == pid).ok_or(2u64)?;
        if staged.started.is_some() {
            return Err(s::launch::STARTED);
        }
        plan_file_service_adoption(&staged.facts)?;
        let reply = self.start_file_transition(s::ADOPT_FILES_V7, Task::Adopt(Adopt::new(pid)))?;
        self.staged = None;
        Ok(reply)
    }
}
