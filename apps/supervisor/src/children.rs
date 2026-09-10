// SPDX-License-Identifier: Apache-2.0
//! Two utility slots, explicit scopes and fresh generations, no inherited shell authority.
use super::services::*;
use rustic_sdk::{
    abi::supervisor as s,
    ipc::{Endpoint, Message},
    process,
    runtime::{self, abi as k},
};
pub struct Child {
    pub pid: u64,
    pub endpoint: Endpoint,
    pub scope: u32,
    pub rights: u8,
    pub expires: u64,
    pub generation: u32,
    pub report: [u64; 7],
    pub closed: bool,
}
impl State {
    pub fn launch(
        &mut self,
        role: u64,
        scope: u32,
        other: u32,
        rights: u8,
        lease: u64,
    ) -> Result<u64, u64> {
        if !matches!(
            role,
            s::SPIN | s::FAULT | s::READ | s::PROBE | s::FINISH | s::WATCH
        ) || lease > 360000
        {
            return Err(1);
        }
        let file_access = matches!(role, s::READ | s::PROBE | s::WATCH);
        if file_access {
            if self.policy == 0 {
                return Err(2);
            }
            if scope == 0 || rights == 0 || rights & !3 != 0 {
                return Err(2);
            }
            self.owner.stat(scope).map_err(|_| 2u64)?;
        } else if scope != 0 || other != 0 || rights != 0 || lease != 0 {
            return Err(1);
        }
        let slot = self.children.iter().position(Option::is_none).ok_or(3u64)?;
        let me = process::id().map_err(|_| 4u64)?;
        let pid = spawn(k::UTILITY).map_err(|_| 3u64)?;
        let mut control = [0; 2];
        let mut data = [0; 2];
        let result = (|| {
            control = connect(me, pid).map_err(|_| 3u64)?;
            let expires = if lease == 0 {
                0
            } else {
                runtime::clock().saturating_add(lease)
            };
            let generation = if file_access {
                data = connect(self.files, pid).map_err(|_| 3u64)?;
                grant(
                    &mut self.admin,
                    slot + 2,
                    pid,
                    data[0],
                    scope,
                    rights,
                    expires,
                )
                .map_err(|_| 2u64)?
            } else {
                0
            };
            let endpoint = Endpoint::from_bootstrap(control[0]);
            endpoint
                .send(
                    &Message::new(
                        0,
                        &k::encode([
                            role,
                            scope as u64,
                            other as u64,
                            generation as u64,
                            0,
                            0,
                            0,
                            0,
                        ]),
                    )
                    .unwrap(),
                )
                .map_err(|_| 4u64)?;
            start(pid, [data[1], control[1], self.files]).map_err(|_| 4u64)?;
            self.children[slot] = Some(Child {
                pid,
                endpoint,
                scope,
                rights,
                expires,
                generation,
                report: [0; 7],
                closed: false,
            });
            Ok(pid)
        })();
        if result.is_err() {
            stop(pid);
            let _ = detach(&mut self.admin, slot + 2);
            close(me, control[0]);
            close(self.files, data[0]);
        }
        result
    }
    pub fn collect(&mut self) {
        for child in self.children.iter_mut().flatten() {
            if child.closed {
                continue;
            }
            match child.endpoint.receive() {
                Ok(message) => {
                    if message.sender() == child.pid
                        && let Ok(w) = k::decode(message.payload())
                    {
                        child.report.copy_from_slice(&w[..7]);
                    }
                }
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {}
                Err(_) => child.closed = true,
            }
        }
    }
    pub fn reap(&mut self, pid: u64) -> Result<[u64; 8], u64> {
        let slot = self
            .children
            .iter()
            .position(|c| c.as_ref().is_some_and(|c| c.pid == pid))
            .ok_or(2u64)?;
        let r = call([k::REAP, pid, 0, 0, 0, 0, 0, 0]).map_err(|_| 3u64)?;
        let child = self.children[slot].take().unwrap();
        let _ = child.endpoint.close();
        let _ = detach(&mut self.admin, slot + 2);
        Ok([0, r[0], r[1], 0, 0, 0, 0, 0])
    }
}
