// SPDX-License-Identifier: Apache-2.0
use crate::arch::{self, Serial};
use core::{fmt::Write, panic::PanicInfo};

pub(crate) const BUILD: &str = match option_env!("RUSTIC_BUILD_ID") {
    Some(value) => value,
    None => "unidentified",
};

pub(crate) fn panic(info: &PanicInfo<'_>) -> ! {
    // If a panic interrupted a UART owner, do not alias it or deadlock on a lock.
    if let Some(mut serial) = Serial::take() {
        let _ = writeln!(serial, "RUSTIC PANIC component=kernel build={BUILD} {info}");
        serial.flush();
    } else {
        // The failing fixture still holds the token. A lost panic message hides
        // which assertion failed, so emit the reason on the raw port instead;
        // this is the terminal path and no owner can use the line afterwards.
        arch::panic_bytes(b"RUSTIC PANIC_UNSERIALIZED component=kernel\n");
        let mut text = [0u8; 512];
        let mut writer = RawText::new(&mut text);
        let _ = write!(writer, "{info}");
        arch::panic_bytes(writer.filled());
    }
    arch::test_exit(0x11)
}

/// Fixed-capacity byte buffer that never allocates and never fails. It stops at
/// a character boundary and translates newlines the way the UART owner does, so
/// a truncated panic line is still readable text rather than partial bytes.
struct RawText<'a> {
    buffer: &'a mut [u8],
    len: usize,
}

impl<'a> RawText<'a> {
    fn new(buffer: &'a mut [u8]) -> Self {
        Self { buffer, len: 0 }
    }
    fn filled(&self) -> &[u8] {
        &self.buffer[..self.len]
    }
    fn push(&mut self, byte: u8) {
        if self.len < self.buffer.len() {
            self.buffer[self.len] = byte;
            self.len += 1;
        }
    }
}

impl core::fmt::Write for RawText<'_> {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        // Keep whole characters: drop trailing continuation bytes that would be
        // split by the remaining capacity.
        let mut end = text.len().min(self.buffer.len() - self.len);
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        for byte in text.as_bytes()[..end].iter().copied() {
            if byte == b'\n' {
                self.push(b'\r');
            }
            self.push(byte);
        }
        Ok(())
    }
}

pub(crate) fn fatal(reason: &str) -> ! {
    if let Some(mut serial) = Serial::take() {
        let _ = writeln!(
            serial,
            "RUSTIC FATAL component=boot build={BUILD} reason={reason}"
        );
        serial.flush();
    }
    arch::test_exit(0x12)
}
