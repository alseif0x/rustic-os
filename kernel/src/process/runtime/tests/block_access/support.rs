// SPDX-License-Identifier: Apache-2.0
use super::super::super::application;
use super::{Exit, Manager, Memory, State};
use rustic_abi::{application::KNOWN, block::ALL};
use rustic_kernel::{block::access::Grant, process::lifecycle::Pid};
static ELF: &[u8] = include_bytes!(concat!(
    env!("RUSTIC_APPLICATION_DIRECTORY"),
    "/block-probe.elf"
));
static MANIFEST: &[u8] = include_bytes!(concat!(
    env!("RUSTIC_APPLICATION_DIRECTORY"),
    "/block-probe.manifest"
));
pub(super) const CAPACITY: u64 = 8_388_608;
#[derive(Default)]
pub(super) struct Stats {
    pub(super) baseline: usize,
    pub(super) peak: usize,
    pub(super) applications: u64,
    pub(super) rejected: u64,
    pub(super) lifecycle: u64,
}
pub(super) fn create(manager: &mut Manager, memory: &mut Memory, stats: &mut Stats) -> Pid {
    let pid =
        application::launch(manager, memory, MANIFEST, "block-probe.elf", ELF, KNOWN).unwrap();
    stats.applications += 1;
    stats.peak = stats.peak.max(stats.baseline - memory.free_frames());
    pid
}
pub(super) fn launch(
    manager: &mut Manager,
    memory: &mut Memory,
    stats: &mut Stats,
    role: u64,
    expected: u64,
) -> (Pid, u64) {
    let pid = create(manager, memory, stats);
    let handle = manager
        .grant_block(
            pid,
            Grant {
                first: 0,
                sectors: CAPACITY,
                rights: ALL,
            },
        )
        .unwrap();
    manager.bootstrap(pid, [handle, role, expected]);
    (pid, handle)
}
pub(super) fn drive(manager: &mut Manager, memory: &mut Memory, pids: &[Pid]) {
    for _ in 0..4096 {
        if pids
            .iter()
            .all(|pid| matches!(manager.state(*pid).unwrap(), State::Exited(_)))
        {
            return;
        }
        assert!(
            manager.step(memory).unwrap().is_some(),
            "control survivor stays ready"
        );
    }
    panic!("block application exceeded event budget");
}
pub(super) fn reap(manager: &mut Manager, memory: &mut Memory, pid: Pid, role: u64) -> u64 {
    let report = manager.process(pid).unwrap().last_report;
    assert_eq!(
        manager.state(pid).unwrap(),
        State::Exited(Exit::Code(0)),
        "role={role} report={report:#x}"
    );
    assert_eq!(report & !0xffff, 0xb100_0000 | (role << 16));
    assert_eq!(manager.wait(memory, pid).unwrap(), Some(Exit::Code(0)));
    report & 0xffff
}
pub(super) fn drain(manager: &mut Manager, memory: &mut Memory) {
    for _ in 0..4096 {
        if manager.block.broker.active().is_none() {
            return;
        }
        manager.step(memory).unwrap();
    }
    panic!("abandoned DMA did not settle");
}
