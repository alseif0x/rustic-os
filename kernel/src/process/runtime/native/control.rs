// SPDX-License-Identifier: Apache-2.0
//! Authenticated bootstrap control. Validate complete output memory before any effect.
use super::{
    super::{manager::Manager, syscall::Action},
    catalog,
};
use crate::arch::{interrupts, memory::Memory};
use rustic_abi::runtime::*;
use rustic_kernel::process::lifecycle::{CAPACITY, Exit, Pid, State};
impl Manager {
    fn owned(&self, pid: u64) -> Result<Pid, Error> {
        let p = Pid(pid);
        let record = self.process(p).map_err(|_| Error::NotFound)?;
        if pid != self.session.supervisor && record.parent != self.session.supervisor {
            return Err(Error::Denied);
        }
        Ok(p)
    }
    pub(in super::super) fn control(&mut self, pid: Pid, memory: &mut Memory) -> Action {
        let mut request_words = None;
        let result = (|| {
            if pid.0 != self.session.supervisor || pid.0 == 0 {
                return Err(Error::Denied);
            }
            let process = self.process(pid).map_err(|_| Error::NotFound)?;
            let [address, length, reserved] = process.frame.arguments();
            if length != 64 || reserved != 0 {
                return Err(Error::Size);
            }
            memory
                .validate_buffer(&process.space, address, 64, true)
                .map_err(|_| Error::Address)?;
            let mut bytes = [0; 64];
            memory
                .copy_from_user(&process.space, address, &mut bytes)
                .map_err(|_| Error::Address)?;
            let words = decode(&bytes)?;
            request_words = Some(words);
            let output = self.operation(words, memory)?;
            let process = self.process(pid).map_err(|_| Error::NotFound)?;
            memory
                .copy_to_user(&process.space, address, &encode(output))
                .map_err(|_| Error::Address)?;
            Ok(64)
        })();
        if result.is_err() && pid.0 != 0 && pid.0 == self.session.supervisor {
            let failed_stage = request_words.is_some_and(|words| {
                matches!(
                    words[0],
                    STAGE_BEGIN | STAGE_COPY | STAGE_COMMIT | STAGE_ABORT
                ) && !self.preserve_stage_on_refusal(words)
            });
            if failed_stage {
                self.clear_image_stage(memory);
            }
        }
        Action::Return(result.unwrap_or_else(Error::code))
    }
    fn operation(&mut self, w: [u64; 8], memory: &mut Memory) -> Result<[u64; 8], Error> {
        validate_control(w)?;
        let mut r = [0; 8];
        match w[0] {
            INFO => {
                r = [
                    VERSION,
                    interrupts::ticks(),
                    memory.free_frames() as u64,
                    CAPACITY as u64,
                    self.processes.iter().flatten().count() as u64,
                    self.broker.counts().0 as u64,
                    self.block.broker.counts().1 as u64,
                    // Heap pages currently mapped by every process that has not
                    // been reaped: an exited one keeps its pages until then.
                    self.processes
                        .iter()
                        .flatten()
                        .map(|process| process.heap.used())
                        .sum(),
                ]
            }
            SPAWN => {
                if !matches!(w[1], FILES | SHELL | UTILITY | TASKS) {
                    return Err(Error::Invalid);
                }
                let child = catalog::launch(self, memory, w[1])?;
                self.table.hold(child).map_err(|_| Error::Busy)?;
                let slot = self.table.slot(child).unwrap();
                let process = self.processes[slot].as_mut().unwrap();
                process.parent = self.session.supervisor;
                process.program = w[1];
                r[0] = child.0;
            }
            START => {
                let child = self.owned(w[1])?;
                if self.state(child).map_err(|_| Error::NotFound)? != State::Dormant {
                    return Err(Error::Busy);
                }
                self.bootstrap(child, [w[2], w[3], w[4]]);
                self.table.start(child).map_err(|_| Error::Busy)?;
            }
            CONNECT => {
                let a = self.owned(w[1])?;
                let b = self.owned(w[2])?;
                let (x, y) = self.connect(a, b).map_err(|_| Error::Full)?;
                r[0] = x;
                r[1] = y;
            }
            BLOCK_GRANT => {
                let p = self.owned(w[1])?;
                let rights = u8::try_from(w[2]).map_err(|_| Error::Invalid)?;
                r[0] = self
                    .grant_block(
                        p,
                        rustic_kernel::block::access::Grant {
                            rights,
                            first: w[3],
                            sectors: w[4],
                        },
                    )
                    .map_err(|_| Error::Denied)?;
            }
            MOVE_ENDPOINT => {
                let owner = self.owned(w[1])?;
                let target = self.owned(w[3])?;
                let rights = u8::try_from(w[4]).map_err(|_| Error::Invalid)?;
                r[0] = self
                    .transfer(owner, w[2], target, rights)
                    .map_err(|_| Error::Denied)?;
            }
            CLOSE_ENDPOINT => {
                let p = self.owned(w[1])?;
                self.broker.close(p.0, w[2]).map_err(|_| Error::Invalid)?;
                self.refresh_waiters();
            }
            CONSOLE_GRANT => {
                if w[1] != 0 {
                    self.live(self.owned(w[1])?).map_err(|_| Error::NotFound)?;
                }
                self.session.console = w[1];
                self.refresh_waiters();
            }
            PROCESS => {
                if w[1] >= CAPACITY as u64 {
                    return Err(Error::Invalid);
                }
                if let Some(p) = self.table.pid_at(w[1] as usize) {
                    let process = self.process(p).map_err(|_| Error::NotFound)?;
                    r[0] = p.0;
                    r[4] = process.preemptions;
                    r[5] = process.parent;
                    r[6] = process.program;
                    match self.state(p).map_err(|_| Error::NotFound)? {
                        State::Dormant => r[1] = 1,
                        State::Ready => r[1] = 2,
                        State::Running => r[1] = 3,
                        State::Blocked => r[1] = 4,
                        State::Exited(exit) => {
                            r[1] = 5;
                            let (kind, code) = exit_words(exit);
                            r[2] = kind;
                            r[3] = code;
                        }
                    }
                }
            }
            KILL | REAP => {
                let p = self.owned(w[1])?;
                if p.0 == self.session.supervisor {
                    return Err(Error::Denied);
                }
                if w[0] == KILL {
                    self.kill(p).map_err(|_| Error::Busy)?;
                } else {
                    let e = self
                        .wait(memory, p)
                        .map_err(|_| Error::NotFound)?
                        .ok_or(Error::Busy)?;
                    let (kind, code) = exit_words(e);
                    r[0] = kind;
                    r[1] = code;
                }
            }
            SHUTDOWN => {
                self.clear_image_stage(memory);
                self.session.shutdown = true;
            }
            #[cfg(feature = "sdk-test")]
            HOLD_COMPLETION => {
                let owner = self.owned(w[1])?;
                self.live(owner).map_err(|_| Error::NotFound)?;
                if !self.block.observation.arm(owner.0, w[2], w[3]) {
                    return Err(Error::Invalid);
                }
            }
            #[cfg(feature = "sdk-test")]
            OBSERVATION_STATUS => r = self.block.observation.status(),
            STAGE_BEGIN | STAGE_COPY | STAGE_COMMIT | STAGE_ABORT => {
                r = self.staging_operation(self.session.supervisor, w, memory)?;
            }
            DEVICE => {
                let g = self.block.geometry().map_err(|_| Error::NotFound)?;
                r[0] = g.sectors;
                r[1] = g.read_only as u64;
            }
            _ => return Err(Error::Invalid),
        }
        Ok(r)
    }
}
fn exit_words(exit: Exit) -> (u64, u64) {
    match exit {
        Exit::Code(code) => (1, code),
        Exit::Fault { vector, .. } => (2, vector),
        Exit::Killed => (3, 0),
    }
}
