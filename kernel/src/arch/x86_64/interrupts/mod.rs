// SPDX-License-Identifier: Apache-2.0
//! One CPU, ring 0: own descriptor tables, PIC/PIT, interrupt-safe clock and waits.
mod clock;
pub(crate) use clock::ticks;
mod dispatch;
mod frame;
mod mask;
mod pic;
mod pit;
mod segments;
mod table;
mod tests;
mod user;
mod user_cpu;

pub(crate) use frame::Frame;
pub(crate) use user::Event as UserEvent;
pub(crate) use user::run as run_user;

use core::{
    marker::PhantomData,
    sync::atomic::{AtomicBool, Ordering},
};
pub(crate) use mask::Mask;
use rustic_kernel::time::Deadline;

core::arch::global_asm!(include_str!("entry.S"));
#[cfg(any(
    target_feature = "sse",
    target_feature = "sse2",
    target_feature = "avx"
))]
compile_error!("Interrupt stubs require the x86_64-unknown-none soft-float, no-SIMD ABI");
static INITIALIZED: AtomicBool = AtomicBool::new(false);

pub(crate) fn emergency_guard() -> u64 {
    segments::emergency_guard()
}

pub(crate) struct Controller {
    _local: PhantomData<*mut ()>,
}

pub(crate) fn initialize() -> Option<Controller> {
    if INITIALIZED.swap(true, Ordering::Relaxed) {
        return None;
    }
    let guard = Mask::acquire();
    user_cpu::initialize();
    // SAFETY: Unique R0 bootstrap CPU, IF=0; static storage lives forever.
    unsafe {
        segments::initialize();
        table::initialize();
    }
    pic::initialize();
    pit::initialize();
    drop(guard);
    // SAFETY: GDT/TSS/IDT and IRQ0 handler are installed before enabling IRQs.
    unsafe { core::arch::asm!("sti", options(nostack)) };
    Some(Controller {
        _local: PhantomData,
    })
}

impl Controller {
    #[cfg(feature = "sdk-test")]
    pub(crate) fn idle(&mut self) {
        let guard = Mask::acquire();
        guard.sleep();
    }

    pub(crate) fn stall(&mut self) -> ! {
        tests::stall(self)
    }
    pub(crate) fn wait_until(&mut self, deadline: Deadline) -> Result<u64, &'static str> {
        let guard = Mask::acquire();
        if !guard.was_enabled() {
            return Err("wait_with_interrupts_disabled");
        }
        while !deadline.reached(clock::ticks()) {
            guard.sleep();
        }
        Ok(clock::ticks())
    }

    pub(crate) fn verify(&mut self) {
        tests::verify(self);
    }
    pub(crate) fn fault(&mut self, mode: rustic_kernel::boot::BootMode) -> ! {
        tests::fault(mode)
    }
}

pub(crate) fn user_guard() -> u64 {
    segments::user_guard()
}
