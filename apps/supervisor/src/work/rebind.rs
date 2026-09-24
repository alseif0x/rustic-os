// SPDX-License-Identifier: Apache-2.0
//! Owner revocation of the shell's V7 file binding: revoke its client slot,
//! then re-run the mount's shell channel and grant phases.
//!
//! The revocation is the file service's: it aborts the slot's open stage
//! without I/O, forgets its receipt and closes the old endpoint before the
//! supervisor sees the reply. Only a confirmed revocation is followed by a new
//! channel and grant; the result is reported with the restart binding words.
use super::super::services::*;
use rustic_sdk::{abi::supervisor as s, runtime::abi as k};
use rustic_supervisor::shell_binding;

pub(super) struct Rebind {
    revoked: bool,
    sent: bool,
    mount: super::mount::Mount,
}

impl Rebind {
    fn new() -> Self {
        Self {
            revoked: false,
            sent: false,
            mount: super::mount::Mount::shell_only(),
        }
    }

    pub fn poll(&mut self, state: &mut State) -> Result<Option<[u64; 8]>, u64> {
        if self.revoked {
            state.work.phase = self.mount.phase();
            return self.mount.poll(state, false, FileProfile::V7);
        }
        state.work.phase = 2;
        if !self.sent {
            match state.admin.begin(&k::encode(shell_binding::revoke())) {
                Ok(()) => self.sent = true,
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {}
                Err(_) => return Err(4),
            }
            return Ok(None);
        }
        let Some(m) = state.admin.poll().map_err(|_| 4u64)? else {
            return Ok(None);
        };
        let reply = k::decode(m.payload()).map_err(|_| 4u64)?;
        if !shell_binding::revoked(reply) {
            return Err(4);
        }
        self.revoked = true;
        Ok(None)
    }

    /// Close a channel created after the revocation that was never reported.
    /// Before the revocation is confirmed nothing was created. After it the
    /// shell has no file binding, so the supervisor stays degraded until a
    /// restart issues a fresh incarnation and binding.
    ///
    /// A revocation sent but not yet acknowledged may still be answered: the
    /// service acknowledges it only after an in-flight admission publication
    /// settles. That late reply is drained so it cannot block the next owner
    /// exchange. The revocation itself may then have taken effect, in which
    /// case the shell's requests fail on its closed endpoint until `restart
    /// files` issues a fresh binding.
    pub fn cancel(&self, state: &mut State) {
        if self.sent && !self.revoked && state.admin.pending() {
            state.mark_admin_drain();
        }
        if self.revoked {
            self.mount.cleanup(state);
            state.degraded = true;
        }
    }
}

impl State {
    /// Owner request [`REVOKE_SHELL_V7`](s::REVOKE_SHELL_V7).
    pub(in super::super) fn revoke_shell_v7(&mut self) -> Result<[u64; 8], u64> {
        if self.profile != FileProfile::V7 {
            return Err(1);
        }
        if self.files == 0
            || self.degraded
            || self.stopping
            || self.admin.pending()
            || self.admin.failed()
            || self.admin_drain
            || !self.work.can_start()
        {
            return Err(3);
        }
        self.work
            .start(s::REVOKE_SHELL_V7, super::Task::Rebind(Rebind::new()))
    }
}
