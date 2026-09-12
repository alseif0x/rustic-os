// SPDX-License-Identifier: Apache-2.0
//! Owner control of two deterministic actors. File policy remains in the file service.
use super::services::{State, call};
use rustic_sdk::{
    abi::supervisor as s,
    runtime::{self, abi as k},
};

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
        if !matches!(
            action,
            s::actor::READ
                | s::actor::STAGE
                | s::actor::COMMIT
                | s::actor::FLOOD
                | s::actor::DRAIN
                | s::actor::STALE
                | s::actor::API_READ
                | s::actor::READ_OPEN
                | s::actor::READ_NEXT
                | s::actor::FILL
                | s::actor::OPERATION_GET
                | s::actor::CAPABILITIES
                | s::actor::PROFILE_GET
                | s::actor::PROFILE_CANCEL
                | s::actor::SELECT_GET
                | s::actor::SELECT_CANCEL
                | s::actor::MISSION_PREPARE
                | s::actor::MISSION_VERIFY
                | s::actor::MISSION_SCHEDULE
                | s::actor::MISSION_INSPECT
                | s::actor::MISSION_CANCEL
        ) {
            return Err(1);
        }
        self.actor_words(pid, [action, 0, 0, 0, 0, 0, 0, 0])
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
            .find(|c| {
                c.pid == pid
                    && matches!(
                        c.role,
                        s::SESSION
                            | s::HELPER
                            | s::ADMISSION_SESSION
                            | s::PRIVATE_ADMISSION_SESSION
                    )
            })
            .ok_or(2u64)?;
        if child.control.pending() {
            return Err(3);
        }
        child.control.begin(&k::encode(words)).map_err(|_| 4u64)?;
        child.actor_state = 1;
        child.actor_deadline = runtime::clock().saturating_add(200);
        child.report = [0; 7];
        Ok([0, 1, 0, 0, 0, 0, 0, 0])
    }
    pub(super) fn actor_status(&self, pid: u64) -> Result<[u64; 8], u64> {
        let c = self
            .children
            .iter()
            .flatten()
            .find(|c| {
                c.pid == pid
                    && matches!(
                        c.role,
                        s::SESSION
                            | s::HELPER
                            | s::ADMISSION_SESSION
                            | s::PRIVATE_ADMISSION_SESSION
                    )
            })
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
