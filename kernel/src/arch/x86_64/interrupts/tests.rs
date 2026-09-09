// SPDX-License-Identifier: Apache-2.0
//! R0 acceptance fixtures. No scheduling or userspace isolation is implied.
use super::{Controller, Mask, clock, dispatch, pic, table};
use crate::arch::Serial;
use core::{fmt::Write, sync::atomic::Ordering};
use rustic_kernel::{
    boot::BootMode,
    time::{Deadline, WaitSet},
};

pub(super) fn verify(controller: &mut Controller) {
    {
        let outer = Mask::acquire();
        let inner = Mask::acquire();
        assert!(outer.was_enabled());
        assert!(!inner.was_enabled());
        assert_eq!(
            controller.wait_until(Deadline::after(clock::ticks(), 0).unwrap()),
            Err("wait_with_interrupts_disabled")
        );
        drop(inner);
        let value: u64;
        let scratch: u64;
        let flags: u64;
        // SAFETY: Owned IDT handles INT3 and synthetic spurious PIC vectors.
        // DF is cleared again before leaving asm, as required by the Rust ABI.
        unsafe {
            core::arch::asm!(
                "std", "int3", "pushfq", "pop {}", "cld", "int 0x27", "int 0x2f",
                out(reg) flags, inout("rax") 0x1234_5678_9abc_def0u64 => value,
                inout("r10") 0xfedc_ba98_7654_3210u64 => scratch,
            );
        }
        assert_eq!(value, 0x1234_5678_9abc_def0);
        assert_eq!(scratch, 0xfedc_ba98_7654_3210);
        assert_ne!(flags & (1 << 10), 0);
        assert_eq!(dispatch::BREAKPOINTS.load(Ordering::Relaxed), 1);
        assert_eq!(dispatch::SPURIOUS.load(Ordering::Relaxed), 2);
    }
    let start = clock::ticks();
    let nanos = clock::nanos();
    let mut waits = WaitSet::<4>::new();
    for (slot, delay) in [5, 9, 9, 17].into_iter().enumerate() {
        waits
            .register(slot, Deadline::after(start, delay).unwrap())
            .unwrap();
    }
    assert!(waits.cancel(3).unwrap());
    let mut completed = [None; 4];
    while let Some(deadline) = waits.next_deadline() {
        let now = controller.wait_until(deadline).unwrap();
        for (slot, due) in waits.complete(now).into_iter().enumerate() {
            if due {
                completed[slot] = Some(now - start);
            }
        }
    }
    assert!((5..=7).contains(&completed[0].unwrap()));
    assert!((9..=11).contains(&completed[1].unwrap()));
    assert_eq!(completed[1], completed[2]);
    assert_eq!(completed[3], None);
    for _ in 0..100 {
        let before = clock::ticks();
        let immediate = controller
            .wait_until(Deadline::after(before, 0).unwrap())
            .unwrap();
        assert!((before..=before + 2).contains(&immediate));
        let after = controller
            .wait_until(Deadline::after(before, 1).unwrap())
            .unwrap();
        assert!((before + 1..=before + 3).contains(&after));
    }
    let end = clock::ticks();
    assert!(clock::nanos() > nanos);
    if let Some(mut serial) = Serial::take() {
        let _ = writeln!(
            serial,
            "RUSTIC IRQ verified=1 breakpoint=1 spurious=2 waits=3 cancelled=1 race_waits=100 ticks={} elapsed_ns={} lateness_max_ticks=2",
            end - start,
            clock::nanos() - nanos
        );
        serial.flush();
    }
}

pub(super) fn fault(mode: BootMode) -> ! {
    let _guard = Mask::acquire();
    if let Some(mut serial) = Serial::take() {
        let _ = writeln!(serial, "RUSTIC FAULT_FIXTURE mode={mode:?}");
        serial.flush();
    }
    match mode {
        BootMode::Exception => {
            // SAFETY: Deliberate #UD, IDT handler must terminate this test image.
            unsafe { core::arch::asm!("ud2", options(noreturn)) }
        }
        BootMode::GeneralProtection | BootMode::DoubleFault => {
            if mode == BootMode::DoubleFault {
                // SAFETY: IF=0; this terminal fixture intentionally faults during #GP delivery.
                unsafe { table::invalidate_gp() };
            }
            // SAFETY: Invalid GDT selector intentionally raises #GP with an error code.
            // With the #GP gate invalidated, the CPU instead delivers #DF on IST1.
            unsafe { core::arch::asm!("mov ax, 0xfff8", "mov ss, ax", "ud2", options(noreturn)) }
        }
        _ => panic!("invalid exception fixture"),
    }
}

pub(super) fn stall(controller: &mut Controller) -> ! {
    controller
        .wait_until(Deadline::after(clock::ticks(), 1).unwrap())
        .unwrap();
    {
        let _guard = Mask::acquire();
        pic::mask_timer_for_test();
    }
    if let Some(mut serial) = Serial::take() {
        let _ = writeln!(serial, "RUSTIC TIMER_STALL reached_wait=1");
        serial.flush();
    }
    controller
        .wait_until(Deadline::after(clock::ticks(), 1).unwrap())
        .unwrap();
    panic!("masked timer unexpectedly completed wait")
}
