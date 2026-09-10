// SPDX-License-Identifier: Apache-2.0
//! Pointer lifetimes end at each syscall. Waits retain only copied integer tokens.
use crate::arch;
pub use rustic_abi::runtime::{self as abi, Error};
pub fn clock() -> u64 {
    // SAFETY: Integer-only query/yield with no retained memory.
    unsafe { arch::call(abi::CLOCK, 0, 0, 0) }
}
pub fn control(words: [u64; 8]) -> Result<[u64; 8], Error> {
    let mut bytes = abi::encode(words);
    // SAFETY: Exclusive initialized in/out buffer remains live for this immediate call.
    let n = Error::decode(unsafe { arch::call(abi::CONTROL, bytes.as_mut_ptr() as u64, 64, 0) })?;
    if n != 64 {
        return Err(Error::Protocol);
    }
    abi::decode(&bytes)
}
pub fn wait_set(handles: &[u64], timeout: u64) -> Result<(), Error> {
    if handles.is_empty() || handles.len() > 8 {
        return Err(Error::Size);
    }
    // SAFETY: The kernel copies this bounded initialized array before blocking.
    let n = Error::decode(unsafe {
        arch::call(
            abi::WAIT_SET,
            handles.as_ptr() as u64,
            handles.len() as u64,
            timeout,
        )
    })?;
    if n != 0 {
        return Err(Error::Protocol);
    }
    Ok(())
}
pub fn console_write(bytes: &[u8]) -> Result<(), Error> {
    for chunk in bytes.chunks(256) {
        // SAFETY: Live immutable input is synchronously copied, never retained.
        let n = Error::decode(unsafe {
            arch::call(
                abi::CONSOLE_WRITE,
                chunk.as_ptr() as u64,
                chunk.len() as u64,
                0,
            )
        })?;
        if n != chunk.len() as u64 {
            return Err(Error::Protocol);
        }
    }
    Ok(())
}
pub fn console_read(bytes: &mut [u8]) -> Result<usize, Error> {
    if bytes.is_empty() || bytes.len() > 64 {
        return Err(Error::Size);
    }
    // SAFETY: Exclusive live output allocation; kernel validates before consuming input.
    let n = Error::decode(unsafe {
        arch::call(
            abi::CONSOLE_READ,
            bytes.as_mut_ptr() as u64,
            bytes.len() as u64,
            0,
        )
    })?;
    if n > bytes.len() as u64 {
        return Err(Error::Protocol);
    }
    Ok(n as usize)
}
pub fn console_wait() -> Result<(), Error> {
    // SAFETY: No pointers; pending state contains no Rust borrow.
    let n = Error::decode(unsafe { arch::call(abi::CONSOLE_WAIT, 0, 0, 0) })?;
    if n != 0 {
        return Err(Error::Protocol);
    }
    Ok(())
}
pub struct Console;
impl core::fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        console_write(s.as_bytes()).map_err(|_| core::fmt::Error)
    }
}
