// SPDX-License-Identifier: Apache-2.0
//! One owner operation at a time; bounded polling leaves independent control available.
mod dispatch;
pub(super) mod launch;
mod mount;
mod policy;
mod restart;
pub(super) mod stage;
pub(super) mod tasks;
use rustic_sdk::runtime;
use rustic_supervisor::jobs::{History, Ticket};
/// Deadline of an ordinary owner job, in 100 Hz PIT ticks.
const DEFAULT_BUDGET_TICKS: u64 = 1000;
// Exactly one job owns its inline, bounded task snapshot. The native supervisor
// has no heap; keeping these <=16 rows here avoids shared/global scratch state.
#[expect(
    clippy::large_enum_variant,
    reason = "one bounded inline job; no native heap"
)]
enum Task {
    Launch(launch::Draft),
    TasksList(tasks::TaskList),
    Restart(restart::Restart),
    StageV7(stage::Stage),
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

    pub fn can_start(&self) -> bool {
        self.active.is_none() && self.history.can_start()
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
            Some(Task::TasksList(task)) => Some(task.slot()),
            _ => None,
        }
    }
    /// Whether the active job may be waiting for a reply on the owner client.
    pub fn reads_owner(&self) -> bool {
        matches!(self.active.as_ref().map(|a| &a.task), Some(Task::StageV7(stage)) if stage.reading())
    }
    fn start(&mut self, kind: u64, task: Task) -> Result<[u64; 8], u64> {
        self.start_budget(kind, task, DEFAULT_BUDGET_TICKS)
    }
    /// Start a job whose deadline is `budget` ticks away. Only a job with a
    /// documented, measured budget uses anything but the default.
    fn start_budget(&mut self, kind: u64, task: Task, budget: u64) -> Result<[u64; 8], u64> {
        let ticket = self.history.start(kind).ok_or(3u64)?;
        self.active = Some(Active {
            ticket,
            task,
            deadline: runtime::clock().saturating_add(budget),
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
