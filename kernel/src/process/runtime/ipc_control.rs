// SPDX-License-Identifier: Apache-2.0
//! Trusted launcher operations and wakeups. No process gets authority by naming a PID.
use super::manager::Manager;
use rustic_abi::ipc::Error;
use rustic_kernel::process::lifecycle::{Pid, State};

impl Manager {
    pub(super) fn live(&self, pid: Pid) -> Result<usize, Error> {
        if matches!(
            self.table.state(pid),
            Ok(State::Dormant | State::Ready | State::Running | State::Blocked)
        ) {
            self.table.slot(pid).map_err(|_| Error::Handle)
        } else {
            Err(Error::Handle)
        }
    }
    pub(super) fn connect(&mut self, a: Pid, b: Pid) -> Result<(u64, u64), Error> {
        self.live(a)?;
        self.live(b)?;
        self.broker.connect(a.0, b.0)
    }
    pub(super) fn transfer(
        &mut self,
        owner: Pid,
        handle: u64,
        target: Pid,
        rights: u8,
    ) -> Result<u64, Error> {
        self.live(owner)?;
        self.live(target)?;
        let token = self.broker.transfer(owner.0, handle, target.0, rights)?;
        self.refresh_waiters();
        Ok(token)
    }
    pub(super) fn bootstrap(&mut self, pid: Pid, args: [u64; 3]) {
        let slot = self.live(pid).expect("live bootstrap process");
        let process = self.processes[slot].as_mut().unwrap();
        assert_eq!(
            process.frame.vector, 0,
            "arguments set only before first entry"
        );
        process.frame.set_arguments(args);
    }
    pub(super) fn cancel_wait(&mut self, pid: Pid) -> Result<(), Error> {
        let slot = self.live(pid)?;
        if self.table.state(pid).unwrap() != State::Blocked {
            return Err(Error::Message);
        }
        let process = self.processes[slot].as_mut().unwrap();
        if !matches!(process.pending, Some(super::record::Pending::Ipc(_))) {
            return Err(Error::Message);
        }
        process.pending = None;
        process.frame.result(Error::Cancelled.code());
        self.table.wake(pid).unwrap();
        Ok(())
    }
}
