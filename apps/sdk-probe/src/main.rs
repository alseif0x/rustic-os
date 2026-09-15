// SPDX-License-Identifier: Apache-2.0
#![no_std]
#![no_main]
mod exchange;
mod heap;
use rustic_sdk::process;
rustic_sdk::entry!(run);

fn run(handle: u64, role: u64, peer: u64) -> u64 {
    if exchange::run(handle, role, peer).is_err() {
        return 1;
    }
    // The bounded memory summary is reported last: the fixture keeps only the
    // most recent report of each process.
    match heap::run() {
        Err(_) => 2,
        Ok(summary) if process::report(summary).is_err() => 3,
        Ok(_) => 0,
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    process::exit(127)
}
