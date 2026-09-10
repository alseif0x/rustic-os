// SPDX-License-Identifier: Apache-2.0
#![no_std]
#![no_main]
mod bootstrap;
mod children;

mod recovery;
mod requests;
mod services;
mod sessions;
mod takeover;
#[cfg(feature = "native")]
mod work;
rustic_sdk::entry!(run);
fn run(initialize: u64, _: u64, _: u64) -> u64 {
    match bootstrap::start(initialize == 1) {
        Ok(mut state) => state.serve(),
        Err(_) => {
            let _ = rustic_sdk::runtime::console_write(
                b"RusticOS: service startup failed; disk preserved.\r\n",
            );
            1
        }
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    rustic_sdk::process::exit(127)
}
