// SPDX-License-Identifier: Apache-2.0
//! Owner operation waiting and monotonic adoption of fresh file bindings.
use super::session::Session;
impl Session {
    pub fn request(&mut self, w: [u64; 8]) -> Result<[u64; 8], super::commands::Error> {
        let r = self
            .supervisor
            .words(w)
            .map_err(|_| super::commands::Error::Service(4))?;
        if matches!(r[0], 0 | 5) {
            Ok(r)
        } else {
            Err(super::commands::Error::Service(r[0]))
        }
    }
    pub fn service(&mut self, w: [u64; 8]) -> Result<[u64; 8], super::commands::Error> {
        let r = self.request(w)?;
        if r[0] == 5 {
            self.wait_job(r[1])
        } else {
            Ok(r)
        }
    }
    pub fn wait_job(&mut self, id: u64) -> Result<[u64; 8], super::commands::Error> {
        use rustic_sdk::{abi::supervisor as s, rpc::Progress};
        loop {
            let r = self.request([s::JOB_STATUS, id, 0, 0, 0, 0, 0, 0])?;
            if r[0] == 0 {
                return self.finish_job(r);
            }
            self.files
                .progress()
                .wait(0)
                .map_err(|_| super::commands::Error::Pending(id))?;
        }
    }
    pub fn finish_job(&mut self, r: [u64; 8]) -> Result<[u64; 8], super::commands::Error> {
        if r[0] != 0 || r[7] != 0 {
            return Err(super::commands::Error::Service(4));
        }
        if r[3] != 0 {
            return Err(super::commands::Error::Service(r[3]));
        }
        if r[2] == rustic_sdk::abi::supervisor::RESTART && r[1] > self.binding_job {
            let generation = u32::try_from(r[6]).map_err(|_| super::commands::Error::Service(4))?;
            if r[4] == 0 || r[5] == 0 || generation == 0 {
                return Err(super::commands::Error::Service(4));
            }
            self.files.rebind(r[5], r[4], generation);
            self.binding_job = r[1];
        }
        Ok([0, r[4], r[5], r[6], 0, 0, 0, 0])
    }
}
