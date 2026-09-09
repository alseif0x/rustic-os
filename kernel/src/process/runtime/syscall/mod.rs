// SPDX-License-Identifier: Apache-2.0
mod ipc;
use super::record::Process;
use crate::arch::memory::Memory;
use rustic_kernel::{ipc::Broker, process::abi};

pub(super) enum Action {
    Resume,
    Return(u64),
    Exit(u64),
    Block(u64),
}
pub(super) fn dispatch(
    process: &mut Process,
    broker: &mut Broker,
    owner: u64,
    memory: &mut Memory,
) -> Action {
    process.calls = process.calls.saturating_add(1);
    let (number, argument) = process.frame.call();
    let result = match number {
        abi::QUERY => abi::VERSION,
        abi::GET_PID => owner,
        abi::EXIT => return Action::Exit(argument),
        abi::REPORT if process.reports < 8 => {
            process.reports += 1;
            process.last_report = argument;
            0
        }
        abi::REPORT => abi::QUOTA,
        4..=8 => return ipc::dispatch(number, process, broker, owner, memory),
        _ => abi::NOT_SUPPORTED,
    };
    Action::Return(result)
}
