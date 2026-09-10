// SPDX-License-Identifier: Apache-2.0
use super::super::{
    record::{Pending, Process},
    syscall::Action,
};
use crate::arch::{interrupts, memory::Memory};
use rustic_abi::runtime::Error;
use rustic_kernel::ipc::Broker;
pub(in super::super) fn readiness(
    broker: &Broker,
    owner: u64,
    handles: &[u64],
    deadline: u64,
) -> Option<u64> {
    // Validate every token even when an earlier channel is already ready.
    let mut ready = false;
    for &h in handles {
        match broker.peek(owner, h) {
            Ok(_) => ready = true,
            Err(rustic_abi::ipc::Error::WouldBlock) => {}
            Err(rustic_abi::ipc::Error::Closed) => return Some(Error::Closed.code()),
            Err(_) => return Some(Error::Invalid.code()),
        }
    }
    (ready || interrupts::ticks() >= deadline).then_some(0)
}
pub(super) fn dispatch(p: &Process, broker: &Broker, owner: u64, memory: &Memory) -> Action {
    let [address, count, timeout] = p.frame.arguments();
    if count == 0 || count > 8 || timeout > 1000 {
        return Action::Return(Error::Size.code());
    }
    let mut bytes = [0; 64];
    if memory
        .copy_from_user(&p.space, address, &mut bytes[..count as usize * 8])
        .is_err()
    {
        return Action::Return(Error::Address.code());
    }
    let mut handles = [0; 8];
    for (h, b) in handles.iter_mut().zip(bytes.as_chunks::<8>().0) {
        *h = u64::from_le_bytes(*b);
    }
    let deadline = interrupts::ticks().saturating_add(timeout);
    match readiness(broker, owner, &handles[..count as usize], deadline) {
        Some(n) => Action::Return(n),
        None => Action::Block(Pending::Any {
            handles,
            count: count as usize,
            deadline,
        }),
    }
}
