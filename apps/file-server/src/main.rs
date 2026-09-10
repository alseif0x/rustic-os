// SPDX-License-Identifier: Apache-2.0
#![no_std]
#![no_main]
mod admin;
mod disk;
mod serving;
rustic_sdk::entry!(run);
fn run(block: u64, admin: u64, initialize: u64) -> u64 {
    let mut disk = disk::Disk::new(block);
    let volume = if initialize == 1 {
        rustic_fs::Volume::initialize(&mut disk)
    } else {
        rustic_fs::Volume::mount(&mut disk)
    };
    let endpoint = rustic_sdk::ipc::Endpoint::from_bootstrap(admin);
    match volume {
        Ok(volume) => serving::run(
            &mut disk,
            rustic_file_service::Server::new(volume),
            endpoint,
        ),
        Err(error) => {
            let code = match error {
                rustic_fs::Error::Empty => 15,
                rustic_fs::Error::Corrupt => 5,
                _ => 2,
            };
            let message = rustic_sdk::ipc::Message::new(
                0,
                &rustic_sdk::abi::runtime::encode([code, 0, 0, 0, 0, 0, 0, 0]),
            )
            .unwrap();
            let _ = endpoint.send(&message);
            code
        }
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    rustic_sdk::process::exit(127)
}
