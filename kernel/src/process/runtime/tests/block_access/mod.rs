// SPDX-License-Identifier: Apache-2.0
mod faults;
mod support;
use super::{Exit, Manager, Memory, State, image};
use crate::arch::Serial;
use core::fmt::Write;
use rustic_kernel::boot::BootMode;
use support::{Stats, drive, launch, reap};

pub(crate) fn verify(memory: &mut Memory, mode: BootMode) {
    let before = memory.free_frames();
    let mut manager = Manager::new();
    manager.block.open(memory).unwrap();
    assert_eq!(before - memory.free_frames(), 3);
    let control = manager.create(memory, image(false), [1, 0, 0]).unwrap();
    let mut stats = Stats {
        baseline: before,
        ..Stats::default()
    };
    let phase = if mode == BootMode::BlockUser {
        let (pid, _) = launch(&mut manager, memory, &mut stats, 0, 0);
        drive(&mut manager, memory, &[pid]);
        let value = reap(&mut manager, memory, pid, 0);
        assert!(value == 1 || value == 2);
        let (pid, _) = launch(&mut manager, memory, &mut stats, 11, value);
        drive(&mut manager, memory, &[pid]);
        let cancelled = reap(&mut manager, memory, pid, 11);
        assert_eq!(cancelled, if value == 1 { 16 } else { 0 });
        for role in 12..=13 {
            let (pid, _) = launch(&mut manager, memory, &mut stats, role, value);
            drive(&mut manager, memory, &[pid]);
            assert_eq!(reap(&mut manager, memory, pid, role), 1);
        }
        let mut serial = Serial::take().unwrap();
        writeln!(
            serial,
            "RUSTIC PUBLICATION verified=1 phase={} cancelled={cancelled} too_late=1 committed=1",
            if value == 1 { "write" } else { "replay" }
        )
        .unwrap();
        writeln!(serial, "RUSTIC ADMISSION verified=1 phase={} admitted=1 cancelled=1 committed=1 replay_writes=0", if value == 1 { "write" } else { "replay" }).unwrap();
        serial.flush();
        if value == 1 { "write" } else { "read" }
    } else {
        faults::verify(&mut manager, memory, &mut stats, control);
        "faults"
    };
    assert_eq!(manager.block.broker.counts(), (0, 0));
    let preemptions = manager.process(control).unwrap().preemptions;
    assert!(preemptions > 0);
    assert!(manager.process(control).unwrap().fixture_progress() > 0);
    manager.kill(control).unwrap();
    assert_eq!(manager.wait(memory, control).unwrap(), Some(Exit::Killed));
    manager.block.shutdown(memory).unwrap();
    assert_eq!(memory.free_frames(), before);
    assert_eq!(manager.broker.counts(), (0, 0));
    let mut serial = Serial::take().unwrap();
    writeln!(serial, "RUSTIC BLOCK_USER verified=1 phase={phase} ring=3 applications={} rejected={} lifecycle={} control_preemptions={preemptions} max_bytes=512 queue_slots=2 handle_slots=4 dma_frames=3 peak_frames={} metadata_bytes={} free_before={before} free_after={}", stats.applications, stats.rejected, stats.lifecycle, stats.peak, core::mem::size_of::<Manager>(), memory.free_frames()).unwrap();
    serial.flush();
}
