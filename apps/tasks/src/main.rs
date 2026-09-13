// SPDX-License-Identifier: Apache-2.0
#![no_std]
#![no_main]

mod preview;
mod read;
mod session;
mod snapshot;

rustic_sdk::entry!(run);

fn run(files: u64, control: u64, peer: u64) -> u64 {
    session::run(files, control, peer)
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    rustic_sdk::process::exit(127)
}
