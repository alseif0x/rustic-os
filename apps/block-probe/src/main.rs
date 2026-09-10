// SPDX-License-Identifier: Apache-2.0
#![no_std]
#![no_main]
mod boundaries;
mod lifecycle;
mod persistence;
mod raw;
use rustic_sdk::{block::Device, process};
rustic_sdk::entry!(run);
fn run(handle: u64, role: u64, expected: u64) -> u64 {
    assert_eq!(Device::version(), Ok(1));
    let device = Device::from_bootstrap(handle);
    let result = match role {
        0 => persistence::run(&device),
        1 => boundaries::run(handle, &device),
        2 => boundaries::foreign(handle),
        3 => boundaries::scoped(&device),
        4..=10 => lifecycle::run(handle, role, expected),
        _ => panic!("unknown fixture role"),
    };
    process::report(0xb100_0000 | (role << 16) | result).unwrap();
    0
}
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    let _ = process::report(
        0xdead_0000
            | info
                .location()
                .map_or(0, |location| u64::from(location.line())),
    );
    process::exit(127)
}
