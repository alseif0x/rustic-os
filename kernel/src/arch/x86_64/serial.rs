// SPDX-License-Identifier: Apache-2.0
use super::io;
use core::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

static CLAIMED: AtomicBool = AtomicBool::new(false);
const COM1: u16 = 0x3f8;

/// Sole owner of R0's polled UART. No locks, interrupts, or allocation.
pub(crate) struct Serial;

impl Serial {
    pub(crate) fn take() -> Option<Self> {
        if CLAIMED.swap(true, Ordering::AcqRel) {
            return None;
        }
        // SAFETY: Single claimed UART in a ring-0, uniprocessor boot environment.
        unsafe {
            io::write_u8(COM1 + 1, 0x00);
            io::write_u8(COM1 + 3, 0x80);
            io::write_u8(COM1, 0x01);
            io::write_u8(COM1 + 1, 0x00);
            io::write_u8(COM1 + 3, 0x03);
            io::write_u8(COM1 + 2, 0xc7);
            io::write_u8(COM1 + 4, 0x0b);
        }
        Some(Self)
    }

    fn byte(&mut self, byte: u8) -> fmt::Result {
        for _ in 0..100_000 {
            // SAFETY: This token exclusively owns the configured R0 UART.
            if unsafe { io::read_u8(COM1 + 5) } & 0x20 != 0 {
                // SAFETY: UART reports transmit capacity; token owns the port.
                unsafe { io::write_u8(COM1, byte) };
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(fmt::Error)
    }

    /// Drain the UART before exiting QEMU. Bounded even if the device fails.
    pub(crate) fn flush(&mut self) {
        for _ in 0..100_000 {
            // SAFETY: The token owns this R0 UART register.
            if unsafe { io::read_u8(COM1 + 5) } & 0x40 != 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }
}

impl fmt::Write for Serial {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for byte in text.bytes() {
            if byte == b'\n' {
                self.byte(b'\r')?;
            }
            self.byte(byte)?;
        }
        Ok(())
    }
}

impl Drop for Serial {
    fn drop(&mut self) {
        CLAIMED.store(false, Ordering::Release);
    }
}
