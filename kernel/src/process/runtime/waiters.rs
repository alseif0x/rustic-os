// SPDX-License-Identifier: Apache-2.0
//! Readiness dispatch for typed IPC and block waits, outside interrupt context.
use super::manager::Manager;
use rustic_abi::ipc::Error;
use rustic_kernel::process::lifecycle::{CAPACITY, State};
impl Manager {
    pub(super) fn refresh_waiters(&mut self) {
        for slot in 0..CAPACITY {
            let Some(pid) = self.table.pid_at(slot) else {
                continue;
            };
            if self.table.state(pid).unwrap() != State::Blocked {
                continue;
            }
            let process = self.processes[slot].as_mut().unwrap();
            let result = match process.pending.expect("blocked process owns a wait") {
                super::record::Pending::Ipc(handle) => match self.broker.peek(pid.0, handle) {
                    Ok(_) => 0,
                    Err(Error::WouldBlock) => continue,
                    Err(error) => error.code(),
                },
                super::record::Pending::Block { handle, id } => {
                    match self.block.broker.wait(pid.0, handle, id) {
                        Ok(()) => 0,
                        Err(rustic_abi::block::Error::WouldBlock) => continue,
                        Err(error) => error.code(),
                    }
                }
            };
            process.pending = None;
            process.frame.result(result);
            self.table.wake(pid).unwrap();
        }
    }
}
