// SPDX-License-Identifier: Apache-2.0
//! Control only the saved admission; no owner-supplied ID or automatic replay.
use super::State;
use rustic_sdk::files::{Client, Error, admission::ActivityPhase};

impl State {
    pub(super) fn schedule(&mut self, files: &mut Client) -> Result<[u64; 8], Error> {
        let id = self.id.ok_or(Error::Invalid)?;
        if self.schedule_attempted {
            return Err(Error::Busy);
        }
        // An uncertain reply must not make a second attempt look like a first.
        self.schedule_attempted = true;
        let status = files.admission_schedule(id)?;
        let phase = match status.phase {
            ActivityPhase::Queued => 4,
            ActivityPhase::Running => 1,
            ActivityPhase::Stopping => 2,
            ActivityPhase::Settling => 3,
        };
        Ok([
            0,
            phase,
            status.cancel_requested as u64,
            status.io_pending as u64,
            0,
            0,
            0,
            0,
        ])
    }

    pub(super) fn inspect(&self, files: &mut Client) -> Result<[u64; 8], Error> {
        files
            .inspect_selected(self.id.ok_or(Error::Invalid)?)
            .map(crate::lifecycle::report)
    }

    pub(super) fn cancel(&mut self, files: &mut Client) -> Result<[u64; 8], Error> {
        let id = self.id.ok_or(Error::Invalid)?;
        if self.cancel_attempted {
            return Err(Error::Busy);
        }
        self.cancel_attempted = true;
        let ack = files.cancel_selected(id)?;
        Ok([0, ack.disposition as u64, 0, 0, 0, 0, 0, 0])
    }
}
