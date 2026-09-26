// SPDX-License-Identifier: Apache-2.0
//! Retain the retiring PID and its device work before issuing a new incarnation.
use super::super::services::*;
use rustic_sdk::runtime::abi as k;
/// Deadline of a V7 start or restart job, in 100 Hz PIT ticks, sized from
/// measurement rather than the ordinary 1,000-tick owner deadline. The V7
/// mount verifies the CRC of every live and retained payload sector, one
/// block request per sector, so its cost grows with allocated payload: under
/// QEMU TCG on the reference machine it took about 0.38 s per MiB, and a
/// volume above roughly 22 MiB of allocated payload timed out under the
/// ordinary deadline. `tools/v7_capacity_test.py` records the mount of a
/// nearly full 64 MiB payload region (130,972 allocated sectors): 2,426 ticks
/// in its first run. This budget is about 2.5 times that measurement. The
/// supervisor console stays responsive, but the file service is unavailable
/// and the single owner job slot is held until the mount ends; an expired job reports a timeout and leaves the
/// supervisor degraded until `restart files`. V5 keeps the default.
pub(super) const V7_BUDGET_TICKS: u64 = 6_000;
pub(super) struct Restart {
    pub initialize: bool,
    profile: FileProfile,
    retired: bool,
    retire: super::retire::Retire,
    mount: super::mount::Mount,
}
impl Restart {
    pub fn new(initialize: bool, profile: FileProfile) -> Self {
        Self {
            initialize,
            profile,
            retired: false,
            retire: super::retire::Retire::new(),
            mount: super::mount::Mount::new(),
        }
    }
    pub fn poll(&mut self, state: &mut State) -> Result<Option<[u64; 8]>, u64> {
        if !self.retired {
            if !self.retire.poll(state)? {
                return Ok(None);
            }
            self.retired = true;
            return Ok(None);
        }
        state.work.phase = self.mount.phase();
        self.mount.poll(state, self.initialize, self.profile)
    }
    pub fn cancel(&self, state: &mut State) {
        if self.retired {
            self.mount.cleanup(state);
            if state.files != 0 {
                let _ = call([k::KILL, state.files, 0, 0, 0, 0, 0, 0]);
            }
        }
        // A failed/timed-out mount keeps its PID for the next explicit drain/retry.
    }
}
