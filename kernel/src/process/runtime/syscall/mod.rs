// SPDX-License-Identifier: Apache-2.0
mod block;
mod ipc;
use super::record::Process;
use crate::arch::memory::Memory;
use rustic_kernel::{ipc::Broker, process::abi};

pub(super) enum Action {
    Resume,
    Return(u64),
    Exit(u64),
    Block(super::record::Pending),
}
pub(super) fn dispatch(
    process: &mut Process,
    broker: &mut Broker,
    block_service: &mut super::block::Service,
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
        9..=14 => return block::dispatch(number, process, block_service, owner, memory),
        _ => abi::NOT_SUPPORTED,
    };
    Action::Return(result)
}
