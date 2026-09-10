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
        let pid = self.launch(s::HELPER, scope, other, 1, 0, root)?;
        Ok([0, pid, 0, 0, 0, 0, 0, 0])
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
        ) {
            return Err(1);
        }
        self.actor_words(pid, [action, 0, 0, 0, 0, 0, 0, 0])
    }
    fn actor_words(&mut self, pid: u64, words: [u64; 8]) -> Result<[u64; 8], u64> {
        let child = self
            .children
            .iter_mut()
            .flatten()
            .find(|c| c.pid == pid && matches!(c.role, s::SESSION | s::HELPER))
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
            .find(|c| c.pid == pid && matches!(c.role, s::SESSION | s::HELPER))
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
