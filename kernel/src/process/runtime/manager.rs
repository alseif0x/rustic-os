// SPDX-License-Identifier: Apache-2.0
use super::{
    Error, loader,
    record::Process,
    syscall::{self, Action},
};
use crate::arch::{interrupts::Frame, memory::Memory};
use rustic_kernel::{
    ipc::Broker,
    process::{
        elf,
        lifecycle::{CAPACITY, Exit, Pid, State, Table},
    },
};

pub(super) struct Manager {
    pub(super) table: Table,
    pub(super) processes: [Option<Process>; CAPACITY],
    pub(super) broker: Broker,
}
impl Manager {
    pub(super) fn new() -> Self {
        Self {
            table: Table::new(),
            processes: [const { None }; CAPACITY],
            broker: Broker::new(),
        }
    }
    pub(super) fn create(
        &mut self,
        memory: &mut Memory,
        bytes: &[u8],
        args: [u64; 3],
    ) -> Result<Pid, Error> {
        let (slot, pid) = self.table.create()?;
        let loaded = match loader::load(memory, bytes) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.table.finish(pid, Exit::Killed)?;
                self.table.reap(pid)?;
                return Err(error);
            }
        };
        self.processes[slot] = Some(Process {
            space: loaded.space,
            frame: Frame::user(loaded.entry, elf::STACK_TOP, args),
            preemptions: 0,
            reports: 0,
            last_report: 0,
            pending: None,
            calls: 0,
        });
        Ok(pid)
    }
    pub(super) fn state(&self, pid: Pid) -> Result<State, Error> {
        Ok(self.table.state(pid)?)
    }
    pub(super) fn process(&self, pid: Pid) -> Result<&Process, Error> {
        Ok(self.processes[self.table.slot(pid)?]
            .as_ref()
            .expect("live slot owns process"))
    }
    /// Execute one event outside IRQ context, then apply lifecycle policy.
    pub(super) fn step(&mut self, memory: &mut Memory) -> Result<Option<Pid>, Error> {
        self.refresh_waiters();
        let Some((slot, pid)) = self.table.schedule()? else {
            return Ok(None);
        };
        let process = self.processes[slot].as_mut().expect("scheduled process");
        let event = memory.run_user(&process.space, &mut process.frame);
        let action = match event.vector {
            32..=47 => {
                if event.vector == 32 {
                    process.preemptions += 1;
                }
                Action::Resume
            }
            128 => syscall::dispatch(process, &mut self.broker, pid.0, memory),
            vector => {
                let exit = Exit::Fault {
                    vector,
                    error: event.error,
                    address: if vector == 14 { event.address } else { 0 },
                };
                self.finish(pid, exit)?;
                return Ok(Some(pid));
            }
        };
        match action {
            Action::Exit(code) => {
                self.finish(pid, Exit::Code(code))?;
                return Ok(Some(pid));
            }
            Action::Return(value) => process.frame.result(value),
            Action::Block(handle) => {
                process.pending = Some(handle);
                self.table.block(pid)?;
            }
            Action::Resume => {}
        }
        if !process.frame.valid_user() {
            self.finish(
                pid,
                Exit::Fault {
                    vector: 13,
                    error: 0,
                    address: 0,
                },
            )?;
        } else if self.table.state(pid)? == State::Running {
            self.table.suspend(pid)?;
        }
        self.refresh_waiters();
        Ok(Some(pid))
    }
    fn finish(&mut self, pid: Pid, exit: Exit) -> Result<(), Error> {
        self.table.finish(pid, exit)?;
        self.processes[self.table.slot(pid)?]
            .as_mut()
            .unwrap()
            .pending = None;
        self.broker.close_owner(pid.0);
        self.refresh_waiters();
        Ok(())
    }
    pub(super) fn kill(&mut self, pid: Pid) -> Result<(), Error> {
        self.finish(pid, Exit::Killed)
    }
    pub(super) fn wait(&mut self, memory: &mut Memory, pid: Pid) -> Result<Option<Exit>, Error> {
        if !matches!(self.table.state(pid)?, State::Exited(_)) {
            return Ok(None);
        }
        let slot = self.table.slot(pid)?;
        let process = self.processes[slot].as_mut().expect("exited process");
        memory
            .destroy_user(&mut process.space)
            .map_err(Error::Memory)?;
        self.processes[slot] = None;
        Ok(Some(self.table.reap(pid)?))
    }
}
