// SPDX-License-Identifier: Apache-2.0
//! Owner control of two deterministic actors. File policy remains in the file service.
use super::services::{State, call};
use rustic_sdk::{abi::supervisor as s, rpc::Rpc, runtime::abi as k};

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
    pub(super) fn revoke_session(&mut self, pid: u64) -> Result<[u64; 8], u64> {
        let slot = self
            .children
            .iter()
            .position(|c| c.as_ref().is_some_and(|c| c.pid == pid))
            .ok_or(2u64)?;
        let root = self.children[slot].as_ref().unwrap().root;
        // Fence issuance locally before waiting for the service acknowledgment.
        // Failure leaves issuance disabled; it never reports successful takeover.
        for child in self
            .children
            .iter_mut()
            .flatten()
            .filter(|c| c.root == root)
        {
            child.rights = 0;
        }
        let r = self
            .admin
            .words([33, (slot + 2) as u64, 0, 0, 0, 0, 0, 0])
            .map_err(|_| 4u64)?;
        if r[0] != 0 {
            return Err(4);
        }
        Ok([0, r[1].count_ones() as u64, r[2], r[3], r[4], 0, 0, 0])
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
        let r = Rpc::new(child.endpoint.token(), pid)
            .words(words)
            .map_err(|_| 4u64)?;
        child.report.copy_from_slice(&r[..7]);
        Ok([0, r[0], r[1], r[2], r[3], r[4], r[5], r[6]])
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
            .find(|h| h.pid == target && h.role == s::HELPER && h.root == c.root)
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
