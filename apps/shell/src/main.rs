// SPDX-License-Identifier: Apache-2.0
#![no_std]
#![no_main]
mod commands;
mod control;
mod output;
#[cfg(feature = "native")]
mod progress;
mod session;
rustic_sdk::entry!(run);
fn run(files: u64, control: u64, generation: u64) -> u64 {
    session::run(files, control, generation as u32)
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    rustic_sdk::process::exit(127)
}
