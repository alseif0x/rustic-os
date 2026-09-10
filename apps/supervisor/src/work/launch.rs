// SPDX-License-Identifier: Apache-2.0
//! Dormant child provisioning. No child runs before an authenticated grant reply.
use super::super::{children::Child, services::*};
use rustic_sdk::{
    abi::supervisor as s,
    ipc::{Endpoint, Message},
    process,
    rpc::Rpc,
    runtime::{self, abi as k},
};
pub(in super::super) struct Draft {
    pub slot: usize,
    pub pid: u64,
    role: u64,
    scope: u32,
    other: u32,
    rights: u8,
    expires: u64,
    pub(super) parent: u32,
    control: [u64; 2],
    data: [u64; 2],
    sent: bool,
}
impl Draft {
    pub(super) fn words(&self) -> [u64; 8] {
        if self.parent != 0 {
            [
                37,
                (self.slot + 2) as u64,
                self.parent as u64,
                self.pid,
                self.data[0],
                self.scope as u64,
                self.rights as u64,
                self.expires,
            ]
        } else {
            [
                32,
                (self.slot + 2) as u64,
                self.pid,
                self.data[0],
                self.scope as u64,
                self.rights as u64,
                self.expires,
                u64::from(matches!(self.role, s::LOST_REPLY | s::LOST_OPERATION)),
            ]
        }
    }
    pub(super) fn poll(&mut self, state: &mut State) -> Result<Option<[u64; 8]>, u64> {
        if !self.sent {
            match state.admin.begin(&k::encode(self.words())) {
                Ok(()) => self.sent = true,
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {
                    return Ok(None);
                }
                Err(_) => return Err(4),
            }
            return Ok(None);
        }
        let Some(message) = state.admin.poll().map_err(|_| 4u64)? else {
            return Ok(None);
        };
        let r = k::decode(message.payload()).map_err(|_| 4u64)?;
        if r[0] != 0 {
            return Err(2);
        }
        let generation = u32::try_from(r[1]).map_err(|_| 4u64)?;
        if generation == 0 || r[2..].iter().any(|v| *v != 0) {
            return Err(4);
        }
        if self.parent != 0
            && !state.children.iter().flatten().any(|c| {
                c.generation == self.parent
                    && c.root == self.parent
                    && c.role == s::SESSION
                    && c.rights != 0
                    && (c.expires == 0 || runtime::clock() < c.expires)
            })
        {
            return Err(2);
        }
        self.activate(state, generation)?;
        Ok(Some([0, self.pid, 0, 0, 0, 0, 0, 0]))
    }
    pub(super) fn cancel(&self, state: &State) {
        stop(self.pid);
        close(process::id().unwrap_or(0), self.control[0]);
        close(state.files, self.data[0]);
    }
    fn activate(&self, state: &mut State, generation: u32) -> Result<(), u64> {
        let endpoint = Endpoint::from_bootstrap(self.control[0]);
        endpoint
            .send(
                &Message::new(
                    0,
                    &k::encode([
                        self.role,
                        self.scope as u64,
                        self.other as u64,
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
        start(self.pid, [self.data[1], self.control[1], state.files]).map_err(|_| 4u64)?;
        state.children[self.slot] = Some(Child {
            pid: self.pid,
            control: Rpc::new(self.control[0], self.pid),
            actor_state: 0,
            actor_deadline: 0,
            scope: self.scope,
            rights: self.rights,
            expires: self.expires,
            generation,
            root: if self.parent == 0 {
                generation
            } else {
                self.parent
            },
            role: self.role,
            file_token: self.data[1],
            report: [0; 7],
            closed: false,
        });
        Ok(())
    }
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
    ) -> Result<[u64; 8], u64> {
        if !matches!(
            role,
            s::SPIN
                | s::FAULT
                | s::READ
                | s::PROBE
                | s::FINISH
                | s::WATCH
                | s::LOST_REPLY
                | s::LOST_OPERATION
                | s::SESSION
                | s::HELPER
        ) || lease > 360000
        {
            return Err(1);
        }
        let file_access = matches!(
            role,
            s::READ
                | s::PROBE
                | s::WATCH
                | s::LOST_REPLY
                | s::LOST_OPERATION
                | s::SESSION
                | s::HELPER
        );
        if file_access {
            if !self.administrative_ready() {
                return Err(3);
            }
            if self.policy == 0
                || scope == 0
                || rights
                    != if matches!(role, s::LOST_REPLY | s::LOST_OPERATION) {
                        7
                    } else if role == s::SESSION {
                        3
                    } else {
                        1
                    }
            {
                return Err(2);
            }
            // The service validates object existence and scope in the actual grant transition.
        } else if scope != 0 || other != 0 || rights != 0 || lease != 0 {
            return Err(1);
        }
        if (role == s::HELPER) != (parent != 0) {
            return Err(2);
        }
        let expires = if parent != 0 {
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
                .expires
        } else if lease == 0 {
            0
        } else {
            runtime::clock().saturating_add(lease)
        };
        let reserved = self.work.reserved();
        let slot = self
            .children
            .iter()
            .enumerate()
            .position(|(i, c)| c.is_none() && Some(i) != reserved)
            .ok_or(3u64)?;
        let me = process::id().map_err(|_| 4u64)?;
        let pid = spawn(k::UTILITY).map_err(|_| 3u64)?;
        let mut d = Draft {
            slot,
            pid,
            role,
            scope,
            other,
            rights,
            expires,
            parent,
            control: [0; 2],
            data: [0; 2],
            sent: false,
        };
        let setup = (|| {
            d.control = connect(me, pid).map_err(|_| 3u64)?;
            if file_access {
                d.data = connect(self.files, pid).map_err(|_| 3u64)?;
            }
            Ok::<(), u64>(())
        })();
        if let Err(error) = setup {
            d.cancel(self);
            return Err(error);
        }
        if file_access {
            self.start_launch(d)
        } else {
            match d.activate(self, 0) {
                Ok(()) => Ok([0, pid, 0, 0, 0, 0, 0, 0]),
                Err(e) => {
                    d.cancel(self);
                    Err(e)
                }
            }
        }
    }
}
