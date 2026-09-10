// SPDX-License-Identifier: Apache-2.0
use super::super::{
    record::{Pending, Process},
    syscall::Action,
};
use crate::arch::{Serial, memory::Memory};
use rustic_abi::runtime::*;
pub(super) fn ready() -> bool {
    Serial::take().is_some_and(|s| s.ready())
}
pub(super) fn dispatch(
    number: u64,
    p: &mut Process,
    owner: u64,
    console: u64,
    memory: &mut Memory,
) -> Action {
    let result = (|| {
        if owner != console {
            return Err(Error::Denied);
        }
        let [address, length, reserved] = p.frame.arguments();
        if number == CONSOLE_WAIT {
            if [address, length, reserved] != [0; 3] {
                return Err(Error::Invalid);
            }
            return Ok(if ready() {
                Action::Return(0)
            } else {
                Action::Block(Pending::Console)
            });
        }
        if reserved != 0 {
            return Err(Error::Invalid);
        }
        let max = if number == CONSOLE_WRITE { 256 } else { 64 };
        if length == 0 || length > max {
            return Err(Error::Size);
        }
        let mut bytes = [0; 256];
        if number == CONSOLE_WRITE {
            memory
                .copy_from_user(&p.space, address, &mut bytes[..length as usize])
                .map_err(|_| Error::Address)?;
            let mut serial = Serial::take().ok_or(Error::Busy)?;
            serial
                .bytes(&bytes[..length as usize])
                .map_err(|_| Error::Busy)?;
            Ok(Action::Return(length))
        } else {
            memory
                .validate_buffer(&p.space, address, length as usize, true)
                .map_err(|_| Error::Address)?;
            let mut serial = Serial::take().ok_or(Error::Busy)?;
            let mut count = 0;
            while count < length as usize {
                let Some(b) = serial.read() else {
                    break;
                };
                bytes[count] = b;
                count += 1;
            }
            if count == 0 {
                return Err(Error::WouldBlock);
            }
            memory
                .copy_to_user(&p.space, address, &bytes[..count])
                .map_err(|_| Error::Address)?;
            Ok(Action::Return(count as u64))
        }
    })();
    result.unwrap_or_else(|e| Action::Return(e.code()))
}
pub(in super::super) fn readiness() -> bool {
    ready()
}
