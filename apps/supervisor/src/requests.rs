// SPDX-License-Identifier: Apache-2.0
//! Owner-only shell requests. Process control never accepts arbitrary catalog/subject authority.
use super::services::*;
use rustic_sdk::{abi::supervisor as s, runtime::abi as k};
impl State {
    pub fn request(&mut self, w: [u64; 8]) -> Result<[u64; 8], u64> {
        let end = match w[0] {
            s::INFO | s::EXIT | s::SERVICES | s::RESTART => 1,
            s::PROCESS | s::KILL | s::REAP | s::PERMISSIONS | s::REVOKE => 2,
            s::RUN => 6,
            _ => return Err(1),
        };
        if w[end..].iter().any(|v| *v != 0) {
            return Err(1);
        }
        match w[0] {
            s::INFO => {
                let r = call([k::INFO, 0, 0, 0, 0, 0, 0, 0]).map_err(|_| 4u64)?;
                Ok([0, r[1], r[2], r[3], r[4], r[5], r[6], 0])
            }
            s::PROCESS => {
                let r = call([k::PROCESS, w[1], 0, 0, 0, 0, 0, 0]).map_err(|_| 1u64)?;
                Ok([0, r[0], r[1], r[2], r[3], r[4], r[5], r[6]])
            }
            s::RESTART => self.restart(),
            s::SERVICES => Ok([0, self.files, self.shell, self.policy as u64, 1, 0, 0, 0]),
            s::RUN => self
                .launch(
                    w[1],
                    u32::try_from(w[2]).map_err(|_| 1u64)?,
                    u32::try_from(w[3]).map_err(|_| 1u64)?,
                    u8::try_from(w[4]).map_err(|_| 1u64)?,
                    w[5],
                )
                .map(|pid| [0, pid, 0, 0, 0, 0, 0, 0]),
            s::KILL => {
                if !self.children.iter().flatten().any(|c| c.pid == w[1]) {
                    return Err(2);
                }
                call([k::KILL, w[1], 0, 0, 0, 0, 0, 0]).map_err(|_| 3u64)?;
                Ok([0; 8])
            }
            s::REAP => self.reap(w[1]),
            s::PERMISSIONS => {
                if w[1] == 0 {
                    return Ok([0, 0, 3, 0, 0, 0, 0, 0]);
                }
                let c = self
                    .children
                    .iter()
                    .flatten()
                    .find(|c| c.pid == w[1])
                    .ok_or(2u64)?;
                Ok([
                    0,
                    c.scope as u64,
                    c.rights as u64,
                    c.generation as u64,
                    c.expires,
                    c.report[0],
                    c.report[1],
                    c.report[2],
                ])
            }
            s::REVOKE => {
                let slot = self
                    .children
                    .iter()
                    .position(|c| c.as_ref().is_some_and(|c| c.pid == w[1]))
                    .ok_or(2u64)?;
                let r = self
                    .admin
                    .words([33, (slot + 2) as u64, 0, 0, 0, 0, 0, 0])
                    .map_err(|_| 4u64)?;
                if r[0] != 0 {
                    return Err(4);
                }
                self.children[slot].as_mut().unwrap().rights = 0;
                Ok([0; 8])
            }
            s::EXIT => {
                call([k::SHUTDOWN, 0, 0, 0, 0, 0, 0, 0]).map_err(|_| 4u64)?;
                Ok([0; 8])
            }
            _ => Err(1),
        }
    }
}
