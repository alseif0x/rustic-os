// SPDX-License-Identifier: Apache-2.0
//! Finite native tasks listing relay and scoped result lifetime.
//!
//! This module owns the one tasks child used by `tasks list`. It deliberately
//! keeps the child protocol private: the shell sees only indexed rows after a
//! complete, authorized snapshot has been assembled.

use super::super::services::{State, stop};
use super::{Task, launch::Draft};
use rustic_sdk::{
    abi::{runtime as k, supervisor as s},
    runtime,
};
use rustic_tasks_contract::{MAX_TASKS, MAX_TITLE, wire};

const RESULT_TICKS: u64 = 1000;

#[derive(Clone, Copy)]
pub(in super::super) struct Cached {
    pub(super) job: u64,
    pub(super) slot: usize,
    pub(super) pid: u64,
    pub(super) scope: u32,
    pub(super) rights: u8,
    pub(super) generation: u32,
    pub(super) expires: u64,
    pub(super) count: usize,
    pub(super) rows: [wire::Row; MAX_TASKS],
    pub(super) deadline: u64,
    pub(super) preview: Option<rustic_tasks_contract::preview::Summary>,
}

#[derive(Clone, Copy)]
enum Phase {
    Launch,
    SendList,
    WaitList,
    SendNext,
    WaitNext,
    Ready,
}

pub(super) struct TaskList {
    draft: Option<Draft>,
    slot: usize,
    pid: u64,
    scope: u32,
    rights: u8,
    expires: u64,
    phase: Phase,
    count: usize,
    rows: [wire::Row; MAX_TASKS],
    deadline: u64,
    request: wire::Request,
    preview: Option<rustic_tasks_contract::preview::Summary>,
}

impl TaskList {
    pub(super) fn slot(&self) -> usize {
        self.slot
    }

    pub(super) fn new(draft: Draft) -> Self {
        let empty = wire::Row {
            id: 0,
            state: rustic_tasks_contract::State::Open,
            title: [0; MAX_TITLE],
            title_len: 0,
        };
        Self {
            slot: draft.slot(),
            pid: draft.pid(),
            scope: draft.scope(),
            rights: draft.rights(),
            expires: draft.expires(),
            draft: Some(draft),
            phase: Phase::Launch,
            count: 0,
            rows: [empty; MAX_TASKS],
            deadline: 0,
            request: wire::Request::List,
            preview: None,
        }
    }

    pub(super) fn poll(&mut self, state: &mut State) -> Result<Option<[u64; 8]>, u64> {
        match self.phase {
            Phase::Launch => {
                let mut draft = self.draft.take().ok_or(4u64)?;
                match draft.poll(state) {
                    Ok(Some(_)) => {
                        self.phase = Phase::SendList;
                        Ok(None)
                    }
                    Ok(None) => {
                        self.draft = Some(draft);
                        Ok(None)
                    }
                    Err(error) => {
                        // Retain the draft so a failed grant/activation can
                        // still close its endpoints and reclaim the dormant
                        // child in the caller's cancellation path.
                        self.draft = Some(draft);
                        Err(error)
                    }
                }
            }
            Phase::SendList => {
                self.begin(state, wire::request_words(self.request))?;
                self.phase = Phase::WaitList;
                Ok(None)
            }
            Phase::WaitList => self.receive(state),
            Phase::SendNext => {
                self.begin(state, [wire::NEXT, 0, 0, 0, 0, 0, 0, 0])?;
                self.phase = Phase::WaitNext;
                Ok(None)
            }
            Phase::WaitNext => self.receive(state),
            Phase::Ready => Ok(Some([0, self.pid, self.count as u64, 0, 0, 0, 0, 0])),
        }
    }

    pub(super) fn cancel(&self, state: &mut State) {
        if let Some(draft) = &self.draft {
            if draft.grant_pending() && state.admin.pending() {
                state.mark_admin_drain();
            }
            draft.cancel(state);
        } else {
            state.dispose_task_child(self.slot, self.pid);
        }
    }

    pub(super) fn cache(&self, job: u64, state: &mut State) -> Option<Cached> {
        if !matches!(self.phase, Phase::Ready) {
            return None;
        }
        let child = state.children[self.slot].as_mut()?;
        if child.pid != self.pid || child.role != s::TASKS || child.closed {
            return None;
        }
        if self.deadline == 0 {
            return None;
        }
        child.actor_state = 2;
        child.actor_deadline = self.deadline;
        Some(Cached {
            job,
            slot: self.slot,
            pid: self.pid,
            scope: self.scope,
            rights: self.rights,
            generation: child.generation,
            expires: self.expires,
            count: self.count,
            rows: self.rows,
            deadline: self.deadline,
            preview: self.preview,
        })
    }

    fn begin(&self, state: &mut State, words: [u64; 8]) -> Result<(), u64> {
        let child = state.children[self.slot].as_mut().ok_or(4u64)?;
        if child.pid != self.pid || child.role != s::TASKS {
            return Err(4);
        }
        child.control.begin(&k::encode(words)).map_err(|_| 4u64)
    }

    fn receive(&mut self, state: &mut State) -> Result<Option<[u64; 8]>, u64> {
        let message = {
            let child = state.children[self.slot].as_mut().ok_or(4u64)?;
            if child.pid != self.pid || child.role != s::TASKS {
                return Err(4);
            }
            child.control.poll().map_err(|_| 4u64)?
        };
        let Some(message) = message else {
            return Ok(None);
        };
        let words = k::decode(message.payload()).map_err(|_| 4u64)?;
        match wire::decode_response(words).ok_or(4u64)? {
            wire::Response::Row(row) => {
                if self.count == MAX_TASKS {
                    return Err(4);
                }
                self.rows[self.count] = row;
                self.count += 1;
                self.phase = Phase::SendNext;
                Ok(None)
            }
            wire::Response::End { count } => {
                if self.request != wire::Request::List
                    || usize::try_from(count).ok() != Some(self.count)
                {
                    return Err(4);
                }
                self.deadline = runtime::clock().saturating_add(RESULT_TICKS);
                self.phase = Phase::Ready;
                Ok(Some([0, self.pid, self.count as u64, 0, 0, 0, 0, 0]))
            }
            wire::Response::PreviewEnd(summary) => {
                if !matches!(self.request, wire::Request::Preview(_))
                    || summary.count as usize != self.count
                {
                    return Err(4);
                }
                self.preview = Some(summary);
                self.deadline = runtime::clock().saturating_add(RESULT_TICKS);
                self.phase = Phase::Ready;
                Ok(Some([0, self.pid, self.count as u64, 0, 0, 0, 0, 0]))
            }
            wire::Response::Invalid => Err(s::tasks::INVALID_DOCUMENT),
            wire::Response::Capacity => Err(s::tasks::CAPACITY_EXCEEDED),
            wire::Response::Service { code } if (1..=31).contains(&code) => {
                Err(s::tasks::FILE_ERROR_BASE + code)
            }
            wire::Response::Service { .. } => Err(4),
        }
    }
}

impl State {
    pub(in super::super) fn task_preview(
        &mut self,
        scope: u32,
        edit: rustic_tasks_contract::preview::Edit,
    ) -> Result<[u64; 8], u64> {
        let started = self.task_list(scope)?;
        // This owner request does not yield: freeze the child request before
        // the first work poll can provision or activate the read-only child.
        if let Some(active) = self.work.active.as_mut()
            && active.ticket.id == started[1]
            && let Task::TasksList(task) = &mut active.task
        {
            task.request = wire::Request::Preview(edit);
            return Ok(started);
        }
        let _ = self.abort_task_list(started[1]);
        Err(4)
    }

    pub(super) fn start_tasks(&mut self, draft: Draft) -> Result<[u64; 8], u64> {
        let task = TaskList::new(draft);
        if self.task_result.is_some() || !self.work.can_start() {
            task.cancel(self);
            return Err(3);
        }
        self.work.start(s::TASKS_LIST, Task::TasksList(task))
    }

    pub(in super::super) fn task_list(&mut self, scope: u32) -> Result<[u64; 8], u64> {
        if scope == 0 || self.task_result.is_some() {
            return Err(3);
        }
        self.launch(s::TASKS, scope, 0, 1, 0, 0)
    }

    pub(in super::super) fn task_row(&mut self, job: u64, index: u64) -> Result<[u64; 8], u64> {
        let Some(cached) = self.task_result.as_ref().copied() else {
            return Err(2);
        };
        if cached.job != job || !self.task_authorized(&cached) {
            self.clear_task_result();
            return Err(2);
        }
        let index = usize::try_from(index).map_err(|_| 1u64)?;
        if index > cached.count {
            return Err(1);
        }
        if index == cached.count {
            self.clear_task_result();
            return Ok(cached
                .preview
                .map_or([0, 0, cached.count as u64, 0, 0, 0, 0, 0], |p| {
                    [
                        0,
                        0,
                        p.count as u64,
                        p.version,
                        p.task_id as u64,
                        p.changed as u64,
                        0,
                        0,
                    ]
                }));
        }
        Ok(rustic_tasks_contract::wire::row_words(&cached.rows[index]))
    }

    pub(in super::super) fn abort_task_list(&mut self, job: u64) -> Result<[u64; 8], u64> {
        if let Some(active) = self.work.active.as_ref()
            && active.ticket.id == job
            && matches!(active.task, Task::TasksList(_))
        {
            let active = self.work.active.take().unwrap();
            if let Task::TasksList(task) = active.task {
                task.cancel(self);
            }
            self.work
                .history
                .finish(active.ticket.id, [6, 0, 0, 0, 0, 0, 0, 0]);
            self.work.phase = 0;
            self.work.io = 0;
            return Ok([0; 8]);
        }
        if self
            .task_result
            .as_ref()
            .is_some_and(|result| result.job == job)
        {
            self.clear_task_result();
            return Ok([0; 8]);
        }
        Err(2)
    }

    pub(in super::super) fn expire_task_result(&mut self) {
        if self
            .task_result
            .as_ref()
            .is_some_and(|result| runtime::clock() >= result.deadline)
        {
            self.clear_task_result();
            #[cfg(feature = "tasks-acceptance")]
            self.acceptance.note_expiry(runtime::clock());
        }
    }

    pub(super) fn clear_task_result(&mut self) {
        let Some(result) = self.task_result.take() else {
            return;
        };
        self.dispose_task_child(result.slot, result.pid);
    }

    pub(super) fn dispose_task_child(&mut self, slot: usize, pid: u64) {
        let Some(current) = self.children.get(slot).and_then(Option::as_ref) else {
            return;
        };
        if current.pid != pid || current.role != s::TASKS {
            return;
        }
        stop(pid);
        let Some(child) = self.children[slot].take() else {
            return;
        };
        if child.pid == pid && child.role == s::TASKS {
            let _ = child.control.endpoint.close();
        } else {
            self.children[slot] = Some(child);
        }
    }

    fn task_authorized(&mut self, cached: &Cached) -> bool {
        if cached.slot >= self.children.len() || runtime::clock() >= cached.deadline {
            return false;
        }
        let Some(child) = self.children[cached.slot].as_mut() else {
            return false;
        };
        if child.pid != cached.pid
            || child.role != s::TASKS
            || child.scope != cached.scope
            || child.rights != cached.rights
            || child.rights == 0
            || child.generation != cached.generation
            || child.expires != cached.expires
            || child.closed
            || child.actor_deadline != cached.deadline
            || (child.expires != 0 && runtime::clock() >= child.expires)
        {
            return false;
        }
        // There is no outstanding RPC after the result is cached. Probe the
        // endpoint once so an externally killed child cannot keep its rows
        // authorized until the result deadline.
        match child.control.endpoint.receive() {
            Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => true,
            Ok(_) => false,
            Err(_) => {
                child.closed = true;
                false
            }
        }
    }

    pub(super) fn mark_admin_drain(&mut self) {
        self.admin_drain = true;
    }

    pub(in super::super) fn poll_admin_drain(&mut self) {
        if !self.admin_drain {
            return;
        }
        match self.admin.poll() {
            Ok(Some(_)) => {
                self.admin_drain = false;
                #[cfg(feature = "tasks-acceptance")]
                self.acceptance.note_admin_drained();
            }
            Ok(None) => {}
            Err(_) => {
                self.admin_drain = false;
                self.degraded = true;
            }
        }
    }
}
