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
    pub control: rustic_sdk::rpc::Rpc,
    pub actor_state: u64,
    pub actor_deadline: u64,
    pub scope: u32,
    pub rights: u8,
    pub expires: u64,
    pub generation: u32,
    pub root: u32,
    pub role: u64,
    pub file_token: u64,
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
        parent: u32,
    ) -> Result<u64, u64> {
        if !matches!(
            role,
            s::SPIN
                | s::FAULT
                | s::READ
                | s::PROBE
                | s::FINISH
                | s::WATCH
                | s::LOST_REPLY
                | s::SESSION
                | s::HELPER
        ) || lease > 360000
        {
            return Err(1);
        }
        let file_access = matches!(
            role,
            s::READ | s::PROBE | s::WATCH | s::LOST_REPLY | s::SESSION | s::HELPER
        );
        if file_access {
            if !self.administrative_ready() {
                return Err(3);
            }
            if self.policy == 0 {
                return Err(2);
            }
            if scope == 0
                || rights
                    != if role == s::LOST_REPLY {
                        7
                    } else if role == s::SESSION {
                        3
                    } else {
                        1
                    }
            {
                return Err(2);
            }
            self.owner.stat(scope).map_err(|_| 2u64)?;
        } else if scope != 0 || other != 0 || rights != 0 || lease != 0 {
            return Err(1);
        }
        if (role == s::HELPER) != (parent != 0) {
            return Err(2);
        }
        let inherited = if parent != 0 {
            Some(
                self.children
                    .iter()
                    .flatten()
                    .find(|c| {
                        c.generation == parent
                            && c.root == parent
                            && c.role == s::SESSION
                            && c.rights != 0
                    })
                    .ok_or(2u64)?
                    .expires,
            )
        } else {
            None
        };
        let slot = self.children.iter().position(Option::is_none).ok_or(3u64)?;
        let me = process::id().map_err(|_| 4u64)?;
        let pid = spawn(k::UTILITY).map_err(|_| 3u64)?;
        let mut control = [0; 2];
        let mut data = [0; 2];
        let result = (|| {
            control = connect(me, pid).map_err(|_| 3u64)?;
            let expires = if let Some(deadline) = inherited {
                deadline
            } else if lease == 0 {
                0
            } else {
                runtime::clock().saturating_add(lease)
            };
            let generation = if file_access {
                data = connect(self.files, pid).map_err(|_| 3u64)?;
                if parent != 0 {
                    let r = self
                        .admin
                        .words([
                            37,
                            (slot + 2) as u64,
                            parent as u64,
                            pid,
                            data[0],
                            scope as u64,
                            rights as u64,
                            expires,
                        ])
                        .map_err(|_| 4u64)?;
                    if r[0] != 0 {
                        return Err(2);
                    }
                    u32::try_from(r[1]).map_err(|_| 4u64)?
                } else if role == s::LOST_REPLY {
                    let r = self
                        .admin
                        .words([
                            32,
                            (slot + 2) as u64,
                            pid,
                            data[0],
                            scope as u64,
                            7,
                            expires,
                            1,
                        ])
                        .map_err(|_| 4u64)?;
                    if r[0] != 0 {
                        return Err(2);
                    }
                    u32::try_from(r[1]).map_err(|_| 4u64)?
                } else {
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
                }
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
                control: rustic_sdk::rpc::Rpc::new(endpoint.token(), pid),
                actor_state: 0,
                actor_deadline: 0,
                scope,
                rights,
                expires,
                generation,
                root: if parent == 0 { generation } else { parent },
                role,
                file_token: data[1],
                report: [0; 7],
                closed: false,
            });
            Ok(pid)
        })();
        if result.is_err() {
            stop(pid);
            if file_access && self.administrative_ready() {
                let _ = detach(&mut self.admin, slot + 2);
            }
            close(me, control[0]);
            close(self.files, data[0]);
        }
        result
    }
    pub fn collect(&mut self) {
        let mut ended_roots = [0; 2];
        let mut ended = 0;
        for child in self.children.iter_mut().flatten() {
            if child.closed {
                continue;
            }
            if matches!(child.role, s::SESSION | s::HELPER) && child.control.pending() {
                match child.control.poll() {
                    Ok(Some(message)) => {
                        if let Ok(w) = k::decode(message.payload()) {
                            child.report.copy_from_slice(&w[..7]);
                            child.actor_state = 2;
                        } else {
                            child.actor_state = 3;
                        }
                    }
                    Ok(None) => {}
                    Err(_) => {
                        child.actor_state = 3;
                    }
                }
                continue;
            }
            // Idle native actors must not send unsolicited control messages. Closed
            // endpoints also reveal root death after a file endpoint has moved.
            match child.control.endpoint.receive() {
                Ok(message) => {
                    if message.sender() == child.pid
                        && let Ok(w) = k::decode(message.payload())
                    {
                        child.report.copy_from_slice(&w[..7]);
                    }
                }
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {}
                Err(_) => {
                    child.closed = true;
                    if child.role == s::SESSION && child.rights != 0 {
                        ended_roots[ended] = child.pid;
                        ended += 1;
                    }
                }
            }
        }
        // The independent control endpoint detects root death even after a file handle moves.
        for pid in &ended_roots[..ended] {
            let _ = self.revoke_session(*pid);
        }
    }
    pub fn reap(&mut self, pid: u64) -> Result<[u64; 8], u64> {
        let slot = self
            .children
            .iter()
            .position(|c| c.as_ref().is_some_and(|c| c.pid == pid))
            .ok_or(2u64)?;
        let r = call([k::REAP, pid, 0, 0, 0, 0, 0, 0]).map_err(|_| 3u64)?;
        if self.children[slot]
            .as_ref()
            .is_some_and(|c| c.role == s::SESSION && c.rights != 0)
        {
            // A pending actor reply can delay closed-endpoint observation by one pass.
            // Keep root identity until its fence has been scheduled, even after a move.
            self.revoke_session(pid)?;
        }
        let child = self.children[slot].take().unwrap();
        let _ = child.control.endpoint.close();
        // Dead client endpoints are also collected by the file server. While admin
        // is unavailable, retain that cleanup there without blocking owner control.
        if child.generation != 0 && self.administrative_ready() {
            let _ = detach(&mut self.admin, slot + 2);
        }
        Ok([0, r[0], r[1], 0, 0, 0, 0, 0])
    }
}
