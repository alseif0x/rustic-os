// SPDX-License-Identifier: Apache-2.0
//! Owner request [`MAINTAIN_V7`](s::MAINTAIN_V7): one administrative exchange
//! asking the V7 file service to maintain retention.
//!
//! The job is owner policy: it starts only from the owner's request, never to
//! make room for a write, and the shell's file grant cannot reach it. The
//! service decides whether it is safe now and refuses with `Busy` while any
//! transfer, stage or unresolved admission is open.
use super::super::services::*;
use rustic_sdk::abi::supervisor as s;
use rustic_supervisor::retention;

impl State {
    pub(in super::super) fn maintain_v7(&mut self) -> Result<[u64; 8], u64> {
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
        self.work.start(
            s::MAINTAIN_V7,
            super::Task::Admin {
                words: retention::request(),
                sent: false,
            },
        )
    }
}
