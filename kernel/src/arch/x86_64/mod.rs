// SPDX-License-Identifier: Apache-2.0
pub(crate) mod interrupts;
mod io;
pub(crate) mod memory;
pub(crate) mod pci;
mod serial;

pub(crate) use serial::Serial;

/// Stop this CPU with maskable interrupts disabled.
pub(crate) fn halt() -> ! {
    loop {
        // SAFETY: Kernel runs at ring 0. This is a terminal path with IF disabled.
        unsafe { core::arch::asm!("cli", "hlt", options(nomem, nostack)) };
    }
}

/// Polled write on the R0 UART for the terminal panic path only. It exists so a
/// panic that interrupts a fixture holding the `Serial` token still reports the
/// reason instead of exiting silently. It never waits forever: a stuck
/// transmitter drops the remaining bytes because the machine is already dying.
pub(crate) fn panic_bytes(bytes: &[u8]) {
    // SAFETY: Ring-0 terminal path. The UART line is either unclaimed or owned by
    // the panicking fixture, which cannot run again; polling cannot deadlock and
    // no other writer is scheduled.
    unsafe {
        for byte in bytes {
            for _ in 0..100_000 {
                if io::read_u8(0x3f8 + 5) & 0x20 != 0 {
                    io::write_u8(0x3f8, *byte);
                    break;
                }
            }
        }
    }
}

/// QEMU test device only. Physical machines need a different shutdown mechanism.
pub(crate) fn test_exit(code: u32) -> ! {
    // SAFETY: R0 reserves 0xf4 for isa-debug-exit; this binary is for R0 only.
    unsafe { io::write_u32(0xf4, code) };
    halt()
}
