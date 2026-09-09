// SPDX-License-Identifier: Apache-2.0
use super::{clock, frame::Frame, pic, segments, user};
use crate::{
    arch::{self, Serial},
    diagnostic,
};
use core::{
    fmt::Write,
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) static BREAKPOINTS: AtomicU64 = AtomicU64::new(0);
pub(super) static SPURIOUS: AtomicU64 = AtomicU64::new(0);

// SAFETY: Unique assembly entry symbol. Stub supplies an aligned live frame and
// preserves all GPRs, DF, IF and the target's no-SIMD ABI. User transitions use the scoped exchange bridge.
#[unsafe(no_mangle)]
extern "C" fn rustic_interrupt(frame: &mut Frame) {
    if (32..=47).contains(&frame.vector) {
        if !pic::acknowledge(frame.vector) {
            SPURIOUS.fetch_add(1, Ordering::Relaxed);
        } else if frame.vector == 32 {
            clock::advance();
        } else {
            fatal(frame);
        }
        if user::dispatch(frame) {
            return;
        }
        return;
    }
    if user::dispatch(frame) {
        return;
    }
    match frame.vector {
        3 => {
            BREAKPOINTS.fetch_add(1, Ordering::Relaxed);
        }
        _ => fatal(frame),
    }
}

fn fatal(frame: &Frame) -> ! {
    let cr2: u64;
    // SAFETY: Ring 0; CR2 is read-only here and useful for page-fault diagnosis.
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags))
    };
    if let Some(mut serial) = Serial::take() {
        let _ = writeln!(
            serial,
            "RUSTIC EXCEPTION build={} vector={} error={:#x} rip={:#x} rsp={:#x} cr2={:#x} emergency={}",
            diagnostic::BUILD,
            frame.vector,
            frame.error,
            frame.rip,
            frame.rsp,
            cr2,
            u8::from(segments::emergency_contains(frame as *const Frame as usize))
        );
        serial.flush();
    }
    arch::test_exit(0x13)
}
