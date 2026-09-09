// SPDX-License-Identifier: Apache-2.0
#![no_std]
#![no_main]

mod arch;
#[path = "boot/entry.rs"]
mod boot;
mod diagnostic;
#[path = "process/runtime/mod.rs"]
mod process;

// SAFETY: Unique ELF entry symbol; Limine supplies the stack and x86_64 state.
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    boot::run()
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    diagnostic::panic(info)
}
