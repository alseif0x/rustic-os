// SPDX-License-Identifier: Apache-2.0
pub(crate) mod interrupts;
mod io;
mod serial;

pub(crate) use serial::Serial;

/// Stop this CPU with maskable interrupts disabled.
pub(crate) fn halt() -> ! {
    loop {
        // SAFETY: Kernel runs at ring 0. This is a terminal path with IF disabled.
        unsafe { core::arch::asm!("cli", "hlt", options(nomem, nostack)) };
    }
}

/// QEMU test device only. Physical machines need a different shutdown mechanism.
pub(crate) fn test_exit(code: u32) -> ! {
    // SAFETY: R0 reserves 0xf4 for isa-debug-exit; this binary is for R0 only.
    unsafe { io::write_u32(0xf4, code) };
    halt()
}
