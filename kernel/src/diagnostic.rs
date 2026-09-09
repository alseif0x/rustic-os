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
    }
    arch::test_exit(0x11)
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
