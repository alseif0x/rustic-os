// SPDX-License-Identifier: Apache-2.0
//! One owner operation at a time; bounded polling leaves independent control available.
mod dispatch;
pub(super) mod launch;
mod mount;
mod policy;
mod restart;
use rustic_sdk::runtime;
use rustic_supervisor::jobs::{History, Ticket};
enum Task {
    Launch(launch::Draft),
    Restart(restart::Restart),
    Admin { words: [u64; 8], sent: bool },
}
pub(super) struct Active {
    ticket: Ticket,
    task: Task,
    deadline: u64,
}
pub(super) struct Work {
    active: Option<Active>,
    history: History,
    pub phase: u64,
    pub io: u64,
}
impl Work {
    pub fn new() -> Self {
        Self {
            active: None,
            history: History::default(),
            phase: 0,
            io: 0,
        }
    }
    pub fn pending(&self) -> bool {
        self.active.is_some()
    }
    pub fn pending_helper(&self, root: u32) -> Option<(usize, u64)> {
        match self.active.as_ref().map(|a| &a.task) {
            Some(Task::Launch(d)) if d.parent == root && root != 0 => Some((d.slot, d.pid)),
            _ => None,
        }
    }
    pub fn reserved(&self) -> Option<usize> {
        match self.active.as_ref().map(|a| &a.task) {
            Some(Task::Launch(d)) => Some(d.slot),
            _ => None,
        }
    }
    fn start(&mut self, kind: u64, task: Task) -> Result<[u64; 8], u64> {
        let ticket = self.history.start(kind).ok_or(3u64)?;
        self.active = Some(Active {
            ticket,
            task,
            deadline: runtime::clock().saturating_add(1000),
        });
        self.phase = 1;
        self.io = 0;
        Ok([5, ticket.id, kind, 1, 0, 0, 0, 0])
    }
    pub fn status(&self, id: u64, server: u64) -> Result<[u64; 8], u64> {
        self.history
            .status(id, self.phase, server, self.io)
            .ok_or(2)
    }
}
