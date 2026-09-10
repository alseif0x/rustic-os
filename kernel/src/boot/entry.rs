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
    let Some(mut interrupts) = arch::interrupts::initialize() else {
        diagnostic::fatal("interrupts_already_initialized")
    };
    let layout = adapter::memory_layout().unwrap_or_else(|reason| diagnostic::fatal(reason));
    let mut memory = arch::memory::Memory::initialize(layout, adapter::memory_regions())
        .unwrap_or_else(|error| panic!("memory initialization: {error:?}"));
    match mode {
        #[cfg(not(feature = "sdk-test"))]
        BootMode::BlockUser | BootMode::BlockUserFaults => {
            panic!("native block acceptance requires sdk-test");
        }
        #[cfg(feature = "sdk-test")]
        BootMode::BlockUser | BootMode::BlockUserFaults => {
            crate::process::verify_block(&mut memory, mode);
            if let Some(mut serial) = Serial::take() {
                writeln!(
                    serial,
                    "RUSTIC SUCCESS component=boot build={}",
                    diagnostic::BUILD
                )
                .unwrap();
                serial.flush();
            }
            arch::test_exit(0x10)
        }
        BootMode::BlockPersist
        | BootMode::BlockReadOnly
        | BootMode::BlockError
        | BootMode::BlockTimeout
        | BootMode::BlockMissing => {
            crate::drivers::block::verify(&mut memory, mode);
            if let Some(mut serial) = Serial::take() {
                writeln!(
                    serial,
                    "RUSTIC SUCCESS component=boot build={}",
                    diagnostic::BUILD
                )
                .unwrap();
                serial.flush();
            }
            arch::test_exit(0x10)
        }
        BootMode::MemoryReadOnly
        | BootMode::MemoryNx
        | BootMode::MemoryUnmapped
        | BootMode::MemoryTextAlias
        | BootMode::MemoryGuard => memory.fault(mode),
        BootMode::TimerStall => interrupts.stall(),
        BootMode::Panic => panic!("deliberate boot test"),
        BootMode::Hang => {
            if let Some(mut serial) = Serial::take() {
                let _ = writeln!(serial, "RUSTIC HANG deliberate=1");
                serial.flush();
            }
            arch::halt()
        }
        BootMode::Ok => {
            interrupts.verify();
            memory.verify();
            crate::process::verify(&mut memory);
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
        BootMode::Exception | BootMode::GeneralProtection | BootMode::DoubleFault => {
            interrupts.fault(mode)
        }
    }
}
