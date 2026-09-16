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
        arch::panic_bytes(b"RUSTIC PANIC_UNSERIALIZED component=kernel\r\n");
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
        // Copy whole characters only. A newline expands to CRLF, so the room
        // required is the encoded character plus one; a character that does not
        // fit is dropped rather than split, and the rest of the line is lost.
        for character in text.chars() {
            let encoded = character.len_utf8();
            let required = encoded + usize::from(character == '\n');
            if self.buffer.len() - self.len < required {
                break;
            }
            if character == '\n' {
                self.push(b'\r');
            }
            let mut bytes = [0u8; 4];
            self.buffer[self.len..self.len + encoded]
                .copy_from_slice(character.encode_utf8(&mut bytes).as_bytes());
            self.len += encoded;
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
