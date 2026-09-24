// SPDX-License-Identifier: Apache-2.0
//! Start of the one staged child under the control-only storage topology.
//!
//! `rustic_supervisor::storage_launch` decides whether the staged manifest and
//! the requested role qualify and what is issued; this module owns the kernel
//! calls and the one control channel. The child stays in the single storage slot
//! (`State::staged`): it never occupies a utility slot, and the owner kills and
//! reaps it through the same staged-child path as before it was started.
//!
//! A refusal before the kernel start leaves the child exactly as staging left
//! it: dormant, with no endpoint. A channel opened for a refused start is closed
//! on both ends, so no channel slot outlives the attempt.
use super::super::services::{BeginError, State, begin, close};
use rustic_sdk::{
    abi::supervisor::launch,
    ipc::Endpoint,
    process,
    runtime::{self, abi as k},
};
use rustic_supervisor::storage_launch::{kernel_refusal, plan};

/// The started staged child's control channel and what it reported on it.
#[derive(Clone, Copy)]
pub(in super::super) struct Started {
    /// The supervisor's end of the control channel.
    control: u64,
    /// The first three words of the child's report, zero until one arrives.
    report: [u64; 3],
    /// The channel reported closed; nothing more will arrive.
    closed: bool,
}

impl Started {
    /// Owner `PERMISSIONS` words: no scope, rights or expiry were issued.
    pub(super) fn facts(&self, generation: u64) -> [u64; 8] {
        [
            0,
            0,
            0,
            generation,
            0,
            self.report[0],
            self.report[1],
            self.report[2],
        ]
    }

    /// Release the supervisor's end once the child has been reaped.
    pub(super) fn close(self) {
        let _ = Endpoint::from_bootstrap(self.control).close();
    }
}

impl State {
    /// Owner request [`START_STAGED`](rustic_sdk::abi::supervisor::START_STAGED).
    pub(in super::super) fn start_staged(&mut self, pid: u64, role: u64) -> Result<[u64; 8], u64> {
        let mut staged = self.staged.filter(|staged| staged.pid == pid).ok_or(2u64)?;
        if self.stopping {
            return Err(3);
        }
        if staged.started.is_some() {
            return Err(launch::STARTED);
        }
        let topology = plan(&staged.facts, role)?;
        let me = process::id().map_err(|_| 4u64)?;
        // The kernel reports `Full` for any refused channel, including one to a
        // child that is no longer live (for example, killed while dormant).
        let r = runtime::control([k::CONNECT, me, pid, 0, 0, 0, 0, 0]).map_err(kernel_refusal)?;
        let control = [r[0], r[1]];
        if let Err(error) = begin(
            control[0],
            pid,
            topology.role_message(),
            topology.start_arguments(control[1]),
        ) {
            // The child did not start: withdraw both ends so it is left dormant
            // with no endpoint and the channel slot is free again.
            close(pid, control[1]);
            close(me, control[0]);
            return Err(match error {
                BeginError::Message => 4,
                BeginError::Start(error) => kernel_refusal(error),
            });
        }
        staged.started = Some(Started {
            control: control[0],
            report: [0; 3],
            closed: false,
        });
        self.staged = Some(staged);
        Ok([0, pid, topology.role(), 0, 0, 0, 0, 0])
    }

    /// Absorb the started staged child's report. Only its own message counts;
    /// only a channel the kernel reports closed ends collection. Any other
    /// receive error leaves the channel observable on the next turn.
    pub(in super::super) fn collect_staged(&mut self) {
        let Some(staged) = self.staged.as_mut() else {
            return;
        };
        let Some(started) = staged.started.as_mut().filter(|started| !started.closed) else {
            return;
        };
        match Endpoint::from_bootstrap(started.control).receive() {
            Ok(message) => {
                if message.sender() == staged.pid
                    && let Ok(words) = k::decode(message.payload())
                {
                    started.report.copy_from_slice(&words[..3]);
                }
            }
            Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::Closed)) => {
                started.closed = true;
            }
            Err(_) => {}
        }
    }

    /// The control end to wait on while the started child may still report.
    pub(in super::super) fn staged_control(&self) -> Option<u64> {
        let started = self.staged?.started?;
        (!started.closed).then_some(started.control)
    }
}
