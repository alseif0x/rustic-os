// SPDX-License-Identifier: Apache-2.0
use crate::arch::{interrupts::Frame, memory::UserSpace};
pub(super) struct Process {
    pub(super) space: UserSpace,
    pub(super) frame: Frame,
    pub(super) preemptions: u64,
    pub(super) reports: u64,
    pub(super) last_report: u64,
    pub(super) pending: Option<u64>,
    pub(super) calls: u64,
}
impl Process {
    pub(super) fn fixture_progress(&self) -> u64 {
        self.frame.registers[3]
    }
}
