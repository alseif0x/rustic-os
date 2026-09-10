// SPDX-License-Identifier: Apache-2.0
//! One asynchronous admin exchange, two retained session records, no grant reissuance while pending.
use super::services::State;
use rustic_sdk::{abi::runtime as k, runtime};
use rustic_supervisor::revocation::Record;
pub(super) struct Takeover {
    records: [Option<Record>; 2],
    active: Option<usize>,
}
impl Takeover {
    pub fn new() -> Self {
        Self {
            records: [None, None],
            active: None,
        }
    }
    pub fn pending(&self) -> bool {
        self.records.iter().flatten().any(Record::pending)
    }
    pub fn status(&self, pid: u64) -> Result<[u64; 8], u64> {
        self.records
            .iter()
            .flatten()
            .find(|r| r.contains(pid))
            .map(|r| r.words(runtime::clock()))
            .ok_or(2)
    }
    pub fn ended(&mut self, server: u64) {
        for record in self.records.iter_mut().flatten() {
            record.service_ended(server);
        }
        self.active = None;
    }
}
impl State {
    pub(super) fn revoke_session(&mut self, pid: u64) -> Result<[u64; 8], u64> {
        let root = self
            .children
            .iter()
            .flatten()
            .find(|c| c.pid == pid)
            .ok_or(2u64)?
            .root;
        if root == 0 {
            return Err(2);
        }
        let mut members = [0; 2];
        let mut mask = 0;
        for (slot, c) in self.children.iter_mut().enumerate() {
            if let Some(c) = c
                && c.root == root
            {
                c.rights = 0;
                members[slot] = c.pid;
                mask |= 1 << (slot + 2);
            }
        }
        if !self
            .takeover
            .records
            .iter()
            .flatten()
            .any(|r| r.server == self.files && r.root == root)
        {
            let slot = self
                .takeover
                .records
                .iter()
                .position(|r| r.is_none_or(|r| !r.pending()))
                .ok_or(3u64)?;
            self.takeover.records[slot] = Some(Record::new(
                self.files,
                root,
                members,
                mask,
                runtime::clock(),
            ));
        }
        self.poll_takeover();
        self.takeover.status(pid)
    }
    pub(super) fn poll_takeover(&mut self) {
        if let Some(index) = self.takeover.active {
            let record = self.takeover.records[index].as_mut().unwrap();
            match self.admin.poll() {
                Ok(Some(message)) => {
                    if let Ok(words) = k::decode(message.payload()) {
                        record.confirm(self.files, words);
                    } else {
                        record.missing();
                    }
                    // Malformed/error acknowledgments require explicit service recovery.
                    if record.pending() {
                        self.degraded = true;
                        return;
                    }
                    self.takeover.active = None;
                    self.degraded =
                        record.words(runtime::clock())[4] != rustic_supervisor::revocation::SETTLED;
                }
                Ok(None) => return,
                Err(_) => {
                    record.missing();
                    self.degraded = true;
                    return;
                }
            }
        }
        if self.admin.pending() || self.admin.failed() || self.stopping {
            return;
        }
        if let Some(index) = self
            .takeover
            .records
            .iter()
            .position(|r| r.is_some_and(|r| r.pending()))
        {
            let record = self.takeover.records[index].as_mut().unwrap();
            match self
                .admin
                .begin(&k::encode([40, record.root as u64, 0, 0, 0, 0, 0, 0]))
            {
                Ok(()) => self.takeover.active = Some(index),
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {}
                Err(_) => {
                    record.missing();
                    self.degraded = true;
                }
            }
        }
    }
    pub(super) fn administrative_ready(&self) -> bool {
        !self.degraded
            && !self.stopping
            && !self.takeover.pending()
            && !self.admin.pending()
            && !self.admin.failed()
    }
}
