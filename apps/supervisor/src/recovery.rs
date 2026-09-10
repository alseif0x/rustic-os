// SPDX-License-Identifier: Apache-2.0
//! Explicit service restart. All utility sessions end; no grants follow a new incarnation.
use super::services::*;
use rustic_sdk::{files::Client, rpc::Rpc, runtime::abi as k};
impl State {
    pub fn restart(&mut self) -> Result<[u64; 8], u64> {
        self.degraded = true;
        self.stopping = true;
        for slot in 0..self.children.len() {
            if let Some(child) = self.children[slot].take() {
                stop(child.pid);
                let _ = child.control.endpoint.close();
            }
        }
        if self.files != 0 {
            let _ = call([k::KILL, self.files, 0, 0, 0, 0, 0, 0]);
            // Retire access only after kernel-confirmed process death, retaining the
            // PID until outstanding driver work settles and reaping succeeds.
            let info = call([k::INFO, 0, 0, 0, 0, 0, 0, 0]).map_err(|_| 4u64)?;
            if info[6] != 0 {
                return Err(3);
            }
            call([k::REAP, self.files, 0, 0, 0, 0, 0, 0]).map_err(|_| 3u64)?;
            self.takeover.ended(self.files);
            self.files = 0;
        }
        let old = core::mem::replace(&mut self.owner, Client::new(0, 0, 0));
        let _ = old.close();
        let old = core::mem::replace(&mut self.admin, Rpc::new(0, 0));
        let _ = old.endpoint.close();
        let (files, admin, owner) = super::bootstrap::file_service(false).map_err(|_| 4u64)?;
        self.files = files;
        self.admin = admin;
        self.owner = owner;
        self.policy = super::policy::load(&mut self.owner).unwrap_or(0);
        let data = connect(files, self.shell).map_err(|_| 4u64)?;
        let generation = owner_grant(&mut self.admin, 0, self.shell, data[0]).map_err(|_| 4u64)?;
        self.degraded = false;
        self.stopping = false;
        Ok([0, files, data[1], generation as u64, 0, 0, 0, 0])
    }
}
