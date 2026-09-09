// SPDX-License-Identifier: Apache-2.0
use crate::{Error, arch, error::decode};
use rustic_abi::process as abi;
pub fn verify_versions() -> Result<(), Error> {
    // SAFETY: Version calls have no pointer arguments.
    let process = unsafe { arch::call(abi::QUERY, 0, 0, 0) };
    // SAFETY: Version calls have no pointer arguments.
    let ipc = unsafe { arch::call(rustic_abi::ipc::INFO, 0, 0, 0) };
    if process == abi::VERSION && ipc == u64::from(rustic_abi::ipc::VERSION) {
        Ok(())
    } else {
        Err(Error::Unsupported)
    }
}
pub fn id() -> Result<u64, Error> {
    // SAFETY: GET_PID has no pointer arguments.
    decode(unsafe { arch::call(abi::GET_PID, 0, 0, 0) })
}
/// Bounded diagnostic integer channel, not a console or file API.
pub fn report(value: u64) -> Result<(), Error> {
    // SAFETY: REPORT accepts one integer.
    if decode(unsafe { arch::call(abi::REPORT, value, 0, 0) })? != 0 {
        return Err(Error::Protocol);
    }
    Ok(())
}
pub fn exit(code: u64) -> ! {
    // SAFETY: EXIT accepts one integer and terminates the current process.
    unsafe {
        arch::call(abi::EXIT, code, 0, 0);
    }
    // An incompatible kernel returning from EXIT must never return into a caller.
    loop {
        core::hint::spin_loop();
    }
}
