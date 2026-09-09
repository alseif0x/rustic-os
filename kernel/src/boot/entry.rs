// SPDX-License-Identifier: Apache-2.0
use crate::{
    arch::{self, Serial},
    diagnostic,
};
use core::fmt::Write;
use rustic_kernel::boot::BootMode;

#[path = "limine.rs"]
mod adapter;

pub(crate) fn run() -> ! {
    {
        let Some(mut serial) = Serial::take() else {
            arch::test_exit(0x12)
        };
        let _ = writeln!(
            serial,
            "RUSTIC START component=boot build={}",
            diagnostic::BUILD
        );
        serial.flush();
    }
    let (summary, mode) = match adapter::inspect() {
        Ok(info) => info,
        Err(reason) => diagnostic::fatal(reason),
    };
    {
        let Some(mut serial) = Serial::take() else {
            arch::test_exit(0x12)
        };
        let _ = writeln!(
            serial,
            "RUSTIC MAP entries={} usable_bytes={}",
            summary.entries, summary.usable_bytes
        );
        serial.flush();
    }
    match mode {
        BootMode::Panic => panic!("deliberate boot test"),
        BootMode::Hang => {
            if let Some(mut serial) = Serial::take() {
                let _ = writeln!(serial, "RUSTIC HANG deliberate=1");
                serial.flush();
            }
            arch::halt()
        }
        BootMode::Ok => {
            if let Some(mut serial) = Serial::take() {
                let _ = writeln!(
                    serial,
                    "RUSTIC SUCCESS component=boot build={}",
                    diagnostic::BUILD
                );
                serial.flush();
            }
            arch::test_exit(0x10)
        }
    }
}
