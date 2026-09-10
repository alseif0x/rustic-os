// SPDX-License-Identifier: Apache-2.0
//! Poll one step, then return to owner control. No nested RPC exchange waits.
use super::super::services::State;
use super::{Task, launch::Draft, restart::Restart};
use rustic_sdk::{
    abi::supervisor as s,
    runtime::{self, abi as k},
};
impl State {
    pub(in super::super) fn start_launch(&mut self, d: Draft) -> Result<[u64; 8], u64> {
        self.work.start(s::RUN, Task::Launch(d))
    }
    pub(in super::super) fn start_restart(&mut self, initialize: bool) -> Result<[u64; 8], u64> {
        if let Some(active) = self.work.active.take() {
            match active.task {
                Task::Launch(d) => d.cancel(self),
                Task::Restart(r) => r.cancel(self),
                Task::Admin { .. } => {}
            }
            self.work
                .history
                .finish(active.ticket.id, [6, 0, 0, 0, 0, 0, 0, 0]);
        }
        self.degraded = true;
        self.stopping = true;
        let roots = self.children.each_ref().map(|c| {
            c.as_ref()
                .filter(|c| c.root != 0 && c.rights != 0)
                .map_or(0, |c| c.pid)
        });
        for pid in roots {
            if pid != 0 {
                self.revoke_session(pid)?;
            }
        }
        for child in &mut self.children {
            if let Some(c) = child.take() {
                super::super::services::stop(c.pid);
                let _ = c.control.endpoint.close();
            }
        }
        self.work
            .start(s::RESTART, Task::Restart(Restart::new(initialize)))
    }
    pub(in super::super) fn start_admin(
        &mut self,
        kind: u64,
        words: [u64; 8],
    ) -> Result<[u64; 8], u64> {
        if !self.administrative_ready() {
            return Err(3);
        }
        self.work.start(kind, Task::Admin { words, sent: false })
    }
    pub(in super::super) fn poll_work(&mut self) {
        let Some(mut active) = self.work.active.take() else {
            return;
        };
        let expired = runtime::clock() >= active.deadline;
        let result = if expired {
            Err(4)
        } else {
            match &mut active.task {
                Task::Launch(d) => d.poll(self),
                Task::Restart(r) => r.poll(self),
                Task::Admin { words, sent } => poll_admin(self, words, sent),
            }
        };
        match result {
            Ok(None) => self.work.active = Some(active),
            Ok(Some(words)) => {
                if active.ticket.kind == s::RESTART {
                    self.degraded = false;
                    self.stopping = false;
                }
                if active.ticket.kind == s::STALL_FILES {
                    self.degraded = true;
                }
                self.work.history.finish(active.ticket.id, words);
            }
            Err(error) => {
                match active.task {
                    Task::Launch(d) => d.cancel(self),
                    Task::Restart(r) => r.cancel(self),
                    Task::Admin { .. } => {}
                }
                if expired || self.admin.failed() {
                    self.degraded = true;
                }
                self.work
                    .history
                    .finish(active.ticket.id, [error, 0, 0, 0, 0, 0, 0, 0]);
            }
        }
    }
}
fn poll_admin(
    state: &mut State,
    words: &[u64; 8],
    sent: &mut bool,
) -> Result<Option<[u64; 8]>, u64> {
    if !*sent {
        match state.admin.begin(&k::encode(*words)) {
            Ok(()) => *sent = true,
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
    if matches!(words[0], 36 | 41) {
        Ok(Some([0, r[0], r[1], 0, 0, 0, 0, 0]))
    } else if r == [0; 8] {
        Ok(Some([0; 8]))
    } else {
        Err(4)
    }
}
