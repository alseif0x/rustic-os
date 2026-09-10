// SPDX-License-Identifier: Apache-2.0
use super::*;
use rustic_kernel::process::elf::{STACK_GUARD, STACK_TOP};

static SENTINEL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0x1234);

pub(super) fn verify(manager: &mut Manager, memory: &mut Memory) -> usize {
    let kernel = &raw const SENTINEL as u64;
    // Victim alone maps 0x700000; attacker maps 0x600000. Two processes using
    // the same normal VA were already checked by the concurrent execution test.
    let cases = [
        ("other-read", 2, 0x700008, 14, 4),
        ("other-write", 3, 0x700008, 14, 6),
        ("kernel-read", 2, kernel, 14, 5),
        ("kernel-write", 3, kernel, 14, 7),
        ("code-write", 3, 0x400000, 14, 7),
        ("stack-execute", 4, STACK_TOP - 4096, 14, 21),
        ("stack-guard", 2, STACK_GUARD, 14, 4),
        ("privileged-cli", 5, 0, 13, 0),
        ("invalid-opcode", 6, 0, 6, 0),
        ("unsupported-fp", 7, 0, 7, 0),
        ("kernel-gate", 8, 0, 13, 0x40a),
        ("invalid-return", 9, 0, 13, 0),
        ("disabled-syscall", 11, 0, 6, 0),
        ("port-io", 12, 0, 13, 0),
        ("privileged-halt", 13, 0, 13, 0),
    ];
    for (name, mode, target, vector, error) in cases {
        let before = memory.free_frames();
        let attacker = manager
            .create(memory, image(false), [mode, target, 999])
            .unwrap();
        let victim = manager.create(memory, image(true), [1, 0, 222]).unwrap();
        drive(manager, memory, attacker);
        assert_eq!(
            manager.wait(memory, attacker).unwrap(),
            Some(Exit::Fault {
                vector,
                error,
                address: if vector == 14 { target } else { 0 }
            }),
            "{name}"
        );
        let previous = manager.process(victim).unwrap().preemptions;
        let progress = manager.process(victim).unwrap().fixture_progress();
        // A timer IRQ can arrive on user entry before the fixture reaches or
        // advances its spin loop. Require actual user progress as well as a
        // preemption within the same bounded event budget.
        for _ in 0..16 {
            manager.step(memory).unwrap();
            let survivor = manager.process(victim).unwrap();
            if survivor.preemptions > previous && survivor.fixture_progress() > progress {
                break;
            }
        }
        assert!(
            manager.process(victim).unwrap().preemptions > previous,
            "{name}: survivor must be timer-preempted after peer fault"
        );
        assert!(
            manager.process(victim).unwrap().fixture_progress() > progress,
            "{name}: survivor must make user progress after peer fault"
        );
        assert_eq!(SENTINEL.load(core::sync::atomic::Ordering::Relaxed), 0x1234);
        manager.kill(victim).unwrap();
        assert_eq!(manager.wait(memory, victim).unwrap(), Some(Exit::Killed));
        assert_eq!(memory.free_frames(), before);
        let mut serial = Serial::take().unwrap();
        writeln!(serial, "RUSTIC PROCESS_FAULT case={name} ring=3 vector={vector} error={error:#x} address={:#x} survivor=1 reclaimed=1", if vector == 14 { target } else { 0 }).unwrap();
        serial.flush();
    }
    cases.len()
}
