// SPDX-License-Identifier: Apache-2.0
//! Owner entry point for asynchronous service recovery.
use super::services::State;
impl State {
    pub fn restart(&mut self) -> Result<[u64; 8], u64> {
        self.begin_restart(false)
    }
    pub(super) fn begin_restart(&mut self, initialize: bool) -> Result<[u64; 8], u64> {
        self.start_restart(initialize)
    }
}
