// SPDX-License-Identifier: Apache-2.0
#![no_std]
#![no_main]
mod exchange;
use rustic_sdk::process;
rustic_sdk::entry!(run);

fn run(handle: u64, role: u64, peer: u64) -> u64 {
    match exchange::run(handle, role, peer) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    process::exit(127)
}
