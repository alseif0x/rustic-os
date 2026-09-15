// SPDX-License-Identifier: Apache-2.0
//! Owner control of the deterministic actors, including the owner-stepped
//! tasks child. File policy remains in the file service.
use super::children::actor_role;
use super::services::{State, call};
use rustic_sdk::{
    abi::supervisor as s,
    runtime::{self, abi as k},
};
use rustic_supervisor::actor;

/// Window for one owner-stepped action that performs a single exchange.
const STEP_TICKS: u64 = 200;
/// Window for a tasks action that performs several file exchanges before it
/// answers; it matches the owner job-status wait used by the tasks clients.
const EXCHANGE_TICKS: u64 = 1100;

impl State {
    pub(super) fn helper(&mut self, parent: u64, scope: u32, other: u32) -> Result<[u64; 8], u64> {
        let root = self
            .children
            .iter()
            .flatten()
            .find(|c| c.pid == parent && c.role == s::SESSION && c.rights != 0)
            .ok_or(2u64)?
            .root;
        self.launch(s::HELPER, scope, other, 1, 0, root)
    }
    pub(super) fn actor(&mut self, pid: u64, action: u64) -> Result<[u64; 8], u64> {
        if !actor::actor_allowed(action) {
            return Err(1);
        }
        // Which of these verbs the addressed child may answer is decided once,
        // by role, in `actor_words`.
        self.actor_words(pid, [action, 0, 0, 0, 0, 0, 0, 0])
    }
    /// Deliver one owner tasks request. The word translation and its shape
    /// checks belong to the library; this keeps only identity and delivery.
    pub(super) fn tasks_owner(&mut self, w: [u64; 8]) -> Result<[u64; 8], u64> {
        let words = rustic_supervisor::tasks_owner::translate(w)?;
        self.actor_words(w[1], words)
    }
    pub(super) fn admission_actor(&mut self, w: [u64; 8]) -> Result<[u64; 8], u64> {
        use rustic_sdk::abi::files::{admission as a, lifecycle};
        // Scheduling/stop/result may be submitted without reading the reply; the discarded
        // acknowledgement is a fault fixture, never an additional authority.
        let modifier = w[6];
        let valid_modifier = match modifier {
            0 => true,
            s::actor::flags::DISCARD_REPLY => {
                matches!(w[5], x if x == a::REQUEST_CANCEL as u64 || x == a::SCHEDULE as u64 || x == a::GET as u64 || x == lifecycle::CANCEL as u64)
            }
            s::actor::flags::OBSERVE_V2 => w[5] == a::OBSERVE as u64,
            s::actor::flags::LIFECYCLE => w[5] == a::OBSERVE as u64,
            s::actor::flags::NEGOTIATED | s::actor::flags::SELECTED => {
                w[5] == a::OBSERVE as u64 || w[5] == lifecycle::CANCEL as u64
            }
            _ => false,
        };
        if w[7] != 0
            || !valid_modifier
            || !matches!(w[5], x if x == a::EXECUTE as u64 || x == a::ACTIVITY as u64 || x == a::REQUEST_CANCEL as u64 || x == a::SCHEDULE as u64 || x == a::GET as u64 || x == a::OBSERVE as u64 || x == lifecycle::CANCEL as u64)
        {
            return Err(1);
        }
        self.actor_words(
            w[1],
            [s::actor::ADMISSION, w[2], w[3], w[4], w[5], modifier, 0, 0],
        )
    }
    fn actor_words(&mut self, pid: u64, words: [u64; 8]) -> Result<[u64; 8], u64> {
        let child = self
            .children
            .iter_mut()
            .flatten()
            .find(|c| c.pid == pid && actor_role(c.role))
            .ok_or(2u64)?;
        // Single place where action and role are checked together: the tasks
        // owner answers only the tasks protocol, and no other role answers it.
        if !actor::role_admits(child.role, words[0]) {
            return Err(1);
        }
        if child.control.pending() {
            return Err(3);
        }
        child.control.begin(&k::encode(words)).map_err(|_| 4u64)?;
        child.actor_state = 1;
        let ticks = match words[0] {
            // These perform several exchanges or syscalls before they answer:
            // two of them address files, and the stress step walks the whole
            // per-process page budget one mapping call at a time.
            s::actor::TASKS_APPLY | s::actor::TASKS_RECOVER | s::actor::TASKS_HEAP_STRESS => {
                EXCHANGE_TICKS
            }
            _ => STEP_TICKS,
        };
        child.actor_deadline = runtime::clock().saturating_add(ticks);
        child.report = [0; 7];
        Ok([0, 1, 0, 0, 0, 0, 0, 0])
    }
    pub(super) fn actor_status(&self, pid: u64) -> Result<[u64; 8], u64> {
        let c = self
            .children
            .iter()
            .flatten()
            .find(|c| c.pid == pid && actor_role(c.role))
            .ok_or(2u64)?;
        let phase = if c.actor_state == 1 && runtime::clock() >= c.actor_deadline {
            3
        } else {
            c.actor_state
        };
        Ok([
            0,
            phase,
            c.report[0],
            c.report[1],
            c.report[2],
            c.report[3],
            c.report[4],
            0,
        ])
    }
    pub(super) fn move_check(&mut self, source: u64, target: u64) -> Result<[u64; 8], u64> {
        let c = self
            .children
            .iter()
            .flatten()
            .find(|c| c.pid == source && c.role == s::SESSION && c.file_token != 0)
            .ok_or(2u64)?;
        let h = self
            .children
            .iter()
            .flatten()
            .find(|h| {
                h.pid == target && h.role == s::HELPER && h.root == c.root && !h.control.pending()
            })
            .ok_or(2u64)?;
        let (token, generation, scope) = (c.file_token, c.generation, c.scope);
        let moved =
            call([k::MOVE_ENDPOINT, source, token, h.pid, 3, 0, 0, 0]).map_err(|_| 4u64)?[0];
        self.children
            .iter_mut()
            .flatten()
            .find(|c| c.pid == source)
            .unwrap()
            .file_token = 0;
        self.actor_words(
            target,
            [
                s::actor::MOVED,
                moved,
                self.files,
                generation as u64,
                scope as u64,
                0,
                0,
                0,
            ],
        )
    }
}
