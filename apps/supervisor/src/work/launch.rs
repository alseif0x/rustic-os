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
use rustic_supervisor::grant::{Phase, Request, Sequence, Step};

/// Bounded extension an expired job is given to withdraw an installed root, in
/// PIT ticks. It is granted once; a channel that does not answer within it leaves
/// the supervisor degraded instead of waiting further.
const WITHDRAWAL_TICKS: u64 = 200;

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
    /// The outstanding reply belongs to an exchange this draft abandoned; it must
    /// be consumed before the administrative channel can carry the withdrawal.
    stale: bool,
    /// The job deadline passed, whatever the sequence still has to finish.
    timeout: bool,
    /// Phase and generation of the private administrative sequence.
    sequence: Sequence,
}
impl Draft {
    pub(super) fn slot(&self) -> usize {
        self.slot
    }
    pub(super) fn pid(&self) -> u64 {
        self.pid
    }
    pub(super) fn scope(&self) -> u32 {
        self.scope
    }
    pub(super) fn rights(&self) -> u8 {
        self.rights
    }
    pub(super) fn expires(&self) -> u64 {
        self.expires
    }

    pub(super) fn grant_pending(&self) -> bool {
        self.sent
    }

    /// The authority this draft asks the service for. The supervisor owns these
    /// facts; the sequence only orders and formats them.
    fn request(&self) -> Request {
        Request {
            slot: self.slot,
            role: self.role,
            peer: self.pid,
            endpoint: self.data[0],
            scope: self.scope,
            other: self.other,
            rights: self.rights,
            expires: self.expires,
            parent: self.parent,
        }
    }

    pub(super) fn words(&self) -> [u64; 8] {
        self.sequence.words(&self.request())
    }

    /// The job deadline passed. The owner result stays a timeout; the returned
    /// grace is the single bounded extension in which an already installed root is
    /// withdrawn instead of being left bound to a child that will never run.
    pub(super) fn expire(&mut self) -> Option<u64> {
        self.timeout = true;
        if self.sequence.phase() == Phase::Withdraw {
            return None;
        }
        match self.sequence.fail(4) {
            Step::Again => {
                self.stale = self.sent;
                Some(WITHDRAWAL_TICKS)
            }
            _ => None,
        }
    }

    pub(super) fn timed_out(&self) -> bool {
        self.timeout
    }

    pub(super) fn poll(&mut self, state: &mut State) -> Result<Option<[u64; 8]>, u64> {
        if self.stale {
            if !state.admin.pending() {
                self.sent = false;
                self.stale = false;
                return Ok(None);
            }
            // The abandoned exchange still owns the administrative channel: its
            // reply is consumed and discarded before the withdrawal is sent.
            let drained = state.admin.poll();
            return match drained {
                Ok(Some(_)) => {
                    self.sent = false;
                    self.stale = false;
                    Ok(None)
                }
                Ok(None) => Ok(None),
                Err(_) => Err(self.abandon(state)),
            };
        }
        if !self.sent {
            let begun = state.admin.begin(&k::encode(self.words()));
            match begun {
                Ok(()) => {
                    self.sent = true;
                    #[cfg(feature = "tasks-acceptance")]
                    if self.role == s::TASKS {
                        state.acceptance.hold_if_armed(self.slot, self.pid);
                    }
                }
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {
                    return Ok(None);
                }
                Err(_) => return Err(self.abandon(state)),
            }
            return Ok(None);
        }
        #[cfg(feature = "tasks-acceptance")]
        if state.acceptance.holds(self.slot, self.pid) {
            return Ok(None);
        }
        let polled = state.admin.poll();
        let reply = match polled {
            Ok(Some(message)) => message,
            Ok(None) => return Ok(None),
            Err(_) => return Err(self.abandon(state)),
        };
        // The exchange has been consumed even when its payload fails
        // validation; cancellation must not try to drain a nonexistent
        // reply or leave the next grant associated with this request.
        self.sent = false;
        let request = self.request();
        let mut step = match k::decode(reply.payload()) {
            Ok(w) => self.sequence.reply(&request, w),
            Err(_) => self.sequence.fail(4),
        };
        if let Step::Ready(generation) = step {
            step = self.admit(state, generation);
        }
        match step {
            // A refused or malformed answer to an installed root leaves a
            // withdrawal owed; the job ends only once it has been answered.
            Step::Again => Ok(None),
            Step::Ready(_) => Ok(Some([0, self.pid, 0, 0, 0, 0, 0, 0])),
            Step::Failed(error) => {
                if self.sequence.leaked() {
                    state.degraded = true;
                }
                Err(error)
            }
        }
    }

    /// Check the last precondition and hand the installed root to the child. Every
    /// refusal here withdraws that root instead of leaving it installed.
    fn admit(&mut self, state: &mut State, generation: u32) -> Step {
        if self.parent != 0
            && !state.children.iter().flatten().any(|c| {
                c.generation == self.parent
                    && c.root == self.parent
                    && c.role == s::SESSION
                    && c.rights != 0
                    && (c.expires == 0 || runtime::clock() < c.expires)
            })
        {
            return self.sequence.fail(2);
        }
        match self.activate(state, generation) {
            Ok(()) => {
                self.sequence.activated();
                Step::Ready(generation)
            }
            Err(error) => self.sequence.fail(error),
        }
    }

    /// The administrative channel failed. Nothing more can be proven about an
    /// installed root, so the job ends and the supervisor is degraded.
    fn abandon(&mut self, state: &mut State) -> u64 {
        let error = self.sequence.abandon(4);
        if self.sequence.leaked() {
            state.degraded = true;
        }
        error
    }

    pub(super) fn cancel(&self, state: &mut State) {
        if self.sent && state.admin.pending() {
            // A reply is still owed on the private channel; no later exchange can
            // use it until that reply has been consumed.
            state.mark_admin_drain();
        } else if self.sequence.outstanding()
            && state
                .admin
                .begin(&k::encode(self.sequence.withdrawal(&self.request())))
                .is_ok()
        {
            // Last resort for a cancellation that will never poll again, such as a
            // service restart: the root is withdrawn without awaiting the answer.
            state.mark_admin_drain();
        }
        if self.sequence.outstanding() {
            state.degraded = true;
        }
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
                | s::LOST_ADMISSION
                | s::ADMISSION_SESSION
                | s::PRIVATE_ADMISSION_SESSION
                | s::SESSION
                | s::HELPER
                | s::TASKS
                | s::TASKS_OWNER
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
                | s::LOST_ADMISSION
                | s::ADMISSION_SESSION
                | s::PRIVATE_ADMISSION_SESSION
                | s::SESSION
                | s::HELPER
                | s::TASKS
                | s::TASKS_OWNER
        );
        if file_access {
            if !self.administrative_ready() {
                return Err(3);
            }
            // The two-scope role must name a second, distinct object; it is the only
            // role for which `other` is granted instead of merely forwarded. That
            // object is also its recovery identity, so it may not be one of the
            // reserved low identifiers: the owner subject `1` is the shell client's.
            if self.policy == 0
                || scope == 0
                || role == s::TASKS_OWNER && (other <= 1 || other == scope)
                || if matches!(role, s::ADMISSION_SESSION | s::PRIVATE_ADMISSION_SESSION) {
                    rights == 0 || rights & !15 != 0
                } else {
                    rights
                        != if matches!(
                            role,
                            s::LOST_REPLY | s::LOST_OPERATION | s::LOST_ADMISSION | s::TASKS_OWNER
                        ) {
                            7
                        } else if role == s::SESSION {
                            3
                        } else {
                            1
                        }
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
        let program = if role == s::TASKS {
            k::TASKS
        } else {
            k::UTILITY
        };
        let pid = spawn(program).map_err(|_| 3u64)?;
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
            stale: false,
            timeout: false,
            sequence: Sequence::new(),
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
            if role == s::TASKS {
                self.start_tasks(d)
            } else {
                self.start_launch(d)
            }
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
