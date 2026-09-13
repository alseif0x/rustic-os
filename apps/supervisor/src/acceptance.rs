// SPDX-License-Identifier: Apache-2.0
//! Guest-only witnesses for the native tasks lifecycle acceptance.
//!
//! This state is a fixture owned by the supervisor.  It never creates a task,
//! changes a grant or synthesizes a result: the hooks only hold the real admin
//! exchange after its send, and record the real cleanup transitions.

use rustic_tasks_contract::acceptance::{self as contract, Request};

#[derive(Clone, Copy)]
struct Held {
    slot: usize,
    pid: u64,
}

#[derive(Default)]
pub(super) struct Fixture {
    armed: bool,
    held: Option<Held>,
    drained: u64,
    expiry_tick: u64,
}

impl Fixture {
    pub(super) fn control(
        &mut self,
        request: Request,
        admin_pending: bool,
    ) -> Result<[u64; 8], u64> {
        match request {
            Request::Arm => self.arm(),
            Request::Status => Ok(self.status(admin_pending)),
            Request::Release => {
                self.armed = false;
                self.held = None;
                Ok([0; 8])
            }
            Request::Reset => {
                // Resetting a live held exchange would discard the witness while
                // leaving the real admin RPC pending.  Release it first, then
                // reset after the owner observes the drain.
                if admin_pending {
                    return Err(3);
                }
                self.armed = false;
                self.held = None;
                self.drained = 0;
                self.expiry_tick = 0;
                Ok([0; 8])
            }
        }
    }

    pub(super) fn hold_if_armed(&mut self, slot: usize, pid: u64) {
        if self.armed && self.held.is_none() {
            self.held = Some(Held { slot, pid });
        }
    }

    pub(super) fn holds(&self, slot: usize, pid: u64) -> bool {
        self.held
            .is_some_and(|held| held.slot == slot && held.pid == pid)
    }

    pub(super) fn note_admin_drained(&mut self) {
        self.drained = self.drained.saturating_add(1);
    }

    pub(super) fn note_expiry(&mut self, tick: u64) {
        self.expiry_tick = tick;
    }

    fn arm(&mut self) -> Result<[u64; 8], u64> {
        if self.armed || self.held.is_some() {
            return Err(3);
        }
        self.armed = true;
        self.drained = 0;
        self.expiry_tick = 0;
        Ok([0; 8])
    }

    fn status(&self, admin_pending: bool) -> [u64; 8] {
        let (held, slot, pid) = self.held.map_or((0, 0, 0), |held| {
            (1, (held.slot as u64).saturating_add(1), held.pid)
        });
        contract::status(
            held,
            u64::from(self.armed),
            slot,
            pid,
            u64::from(admin_pending),
            self.drained,
            self.expiry_tick,
        )
    }
}
