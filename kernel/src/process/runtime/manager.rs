// SPDX-License-Identifier: Apache-2.0
use super::{Error, loader};
use crate::arch::{
    interrupts::Frame,
    memory::{Memory, UserSpace},
};
use rustic_kernel::process::{
    abi, elf,
    lifecycle::{CAPACITY, Exit, Pid, State, Table},
};

pub(super) struct Process {
    space: UserSpace,
    frame: Frame,
    pub(super) preemptions: u64,
    pub(super) reports: u64,
    pub(super) last_report: u64,
}

impl Process {
    pub(super) fn fixture_progress(&self) -> u64 {
        self.frame.registers[3]
    }
}

pub(super) struct Manager {
    table: Table,
    processes: [Option<Process>; CAPACITY],
}

impl Manager {
    pub(super) fn new() -> Self {
        Self {
            table: Table::new(),
            processes: [const { None }; CAPACITY],
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

    /// Run one selected process until timer, syscall or synchronous exception.
    /// Each event ends this quantum. No allocation/lifecycle policy runs in IRQ.
    pub(super) fn step(&mut self, memory: &Memory) -> Result<Option<Pid>, Error> {
        let Some((slot, pid)) = self.table.schedule()? else {
            return Ok(None);
        };
        let process = self.processes[slot]
            .as_mut()
            .expect("scheduled slot owns process");
        let event = memory.run_user(&process.space, &mut process.frame);
        let exit = match event.vector {
            32..=47 => {
                if event.vector == 32 {
                    process.preemptions += 1;
                }
                None
            }
            128 => {
                let (number, argument) = process.frame.call();
                let result = match number {
                    abi::QUERY => abi::VERSION,
                    abi::GET_PID => pid.0,
                    abi::EXIT => {
                        self.table.finish(pid, Exit::Code(argument))?;
                        return Ok(Some(pid));
                    }
                    abi::REPORT if process.reports < 8 => {
                        process.reports += 1;
                        process.last_report = argument;
                        0
                    }
                    abi::REPORT => abi::QUOTA,
                    _ => abi::NOT_SUPPORTED,
                };
                process.frame.result(result);
                None
            }
            vector => Some(Exit::Fault {
                vector,
                error: event.error,
                address: if vector == 14 { event.address } else { 0 },
            }),
        };
        if let Some(exit) = exit {
            self.table.finish(pid, exit)?;
        } else if !process.frame.valid_user() {
            self.table.finish(
                pid,
                Exit::Fault {
                    vector: 13,
                    error: 0,
                    address: 0,
                },
            )?;
        } else {
            self.table.suspend(pid)?;
        }
        Ok(Some(pid))
    }

    pub(super) fn kill(&mut self, pid: Pid) -> Result<(), Error> {
        self.table.finish(pid, Exit::Killed)?;
        Ok(())
    }

    /// Nonblocking wait/reap. The caller drives scheduling while this is pending.
    pub(super) fn wait(&mut self, memory: &mut Memory, pid: Pid) -> Result<Option<Exit>, Error> {
        if !matches!(self.table.state(pid)?, State::Exited(_)) {
            return Ok(None);
        }
        let slot = self.table.slot(pid)?;
        let process = self.processes[slot]
            .as_mut()
            .expect("exited slot owns process");
        memory
            .destroy_user(&mut process.space)
            .map_err(Error::Memory)?;
        self.processes[slot] = None;
        Ok(Some(self.table.reap(pid)?))
    }
}
