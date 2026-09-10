// SPDX-License-Identifier: Apache-2.0
use crate::arch::{interrupts::Frame, memory::UserSpace};
#[derive(Clone, Copy)]
pub(super) enum Pending {
    Ipc(u64),
    #[cfg(feature = "sdk-test")]
    Console,
    #[cfg(feature = "sdk-test")]
    Any {
        handles: [u64; 8],
        count: usize,
        deadline: u64,
    },
    Block {
        handle: u64,
        id: u64,
    },
}
pub(super) struct Process {
    #[cfg(feature = "sdk-test")]
    pub(super) parent: u64,
    #[cfg(feature = "sdk-test")]
    pub(super) program: u64,
    pub(super) space: UserSpace,
    pub(super) frame: Frame,
    pub(super) preemptions: u64,
    pub(super) reports: u64,
    pub(super) last_report: u64,
    pub(super) pending: Option<Pending>,
    pub(super) calls: u64,
}
impl Process {
    pub(super) fn fixture_progress(&self) -> u64 {
        self.frame.registers[3]
    }
}
