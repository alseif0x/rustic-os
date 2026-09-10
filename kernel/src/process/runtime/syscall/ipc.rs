// SPDX-License-Identifier: Apache-2.0
use super::{Action, Process};
use crate::arch::memory::Memory;
use rustic_abi::ipc::*;
use rustic_kernel::ipc::Broker;

pub(super) fn dispatch(
    number: u64,
    process: &mut Process,
    broker: &mut Broker,
    owner: u64,
    memory: &mut Memory,
) -> Action {
    let [handle, address, length] = process.frame.arguments();
    if number == INFO {
        return Action::Return(u64::from(VERSION));
    }
    if number == WAIT {
        return match broker.peek(owner, handle) {
            Ok(_) => Action::Return(0),
            Err(Error::WouldBlock) => Action::Block(super::super::record::Pending::Ipc(handle)),
            Err(error) => Action::Return(error.code()),
        };
    }
    let result = (|| {
        if number == CLOSE {
            broker.close(owner, handle)?;
            return Ok(0);
        }
        broker.check(owner, handle, if number == SEND { WRITE } else { READ })?;
        if !(HEADER as u64..=MAX_MESSAGE as u64).contains(&length) {
            return Err(Error::Size);
        }
        if number == SEND {
            let mut bytes = [0; MAX_MESSAGE];
            memory
                .copy_from_user(&process.space, address, &mut bytes[..length as usize])
                .map_err(|_| Error::Address)?;
            broker.send(owner, handle, &bytes[..length as usize])?;
            Ok(0)
        } else {
            let message = broker.peek(owner, handle)?;
            if length < message.length() as u64 {
                return Err(Error::Size);
            }
            let bytes = message.encode();
            memory
                .copy_to_user(&process.space, address, &bytes[..message.length()])
                .map_err(|_| Error::Address)?;
            broker.consume(owner, handle)?;
            Ok(message.length() as u64)
        }
    })();
    Action::Return(result.unwrap_or_else(Error::code))
}
