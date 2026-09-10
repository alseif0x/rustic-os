// SPDX-License-Identifier: Apache-2.0
#![no_std]
#![no_main]
mod actions;
mod pressure;
mod read;
mod recovery;
mod session;
rustic_sdk::entry!(run);
fn run(files: u64, control: u64, peer: u64) -> u64 {
    let endpoint = rustic_sdk::ipc::Endpoint::from_bootstrap(control);
    if endpoint.wait().is_err() {
        return 1;
    }
    let Ok(message) = endpoint.receive() else {
        return 2;
    };
    let Ok(words) = rustic_sdk::abi::runtime::decode(message.payload()) else {
        return 3;
    };
    let mut client = rustic_sdk::files::Client::new(files, peer, words[3] as u32);
    if matches!(
        words[0],
        rustic_sdk::abi::supervisor::SESSION | rustic_sdk::abi::supervisor::HELPER
    ) {
        return session::run(
            &mut client,
            &endpoint,
            message.sender(),
            words[1] as u32,
            words[2] as u32,
        );
    }
    let report = actions::run(&mut client, words);
    let message =
        rustic_sdk::ipc::Message::new(0, &rustic_sdk::abi::runtime::encode(report)).unwrap();
    let _ = endpoint.send(&message);
    report[0]
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    rustic_sdk::process::exit(127)
}
