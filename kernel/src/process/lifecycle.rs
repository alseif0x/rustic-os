// SPDX-License-Identifier: Apache-2.0
//! Fixed-capacity round robin; PID allocation is monotonic and fails on overflow.
pub const CAPACITY: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pid(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Code(u64),
    Fault {
        vector: u64,
        error: u64,
        address: u64,
    },
    Killed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Ready,
    Running,
    Exited(Exit),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Full,
    Unknown,
    Running,
    NotExited,
    AlreadyExited,
    NotRunning,
    Exhausted,
}

#[derive(Debug, Clone, Copy)]
struct Record {
    pid: Pid,
    state: State,
}

pub struct Table {
    slots: [Option<Record>; CAPACITY],
    next: u64,
    cursor: usize,
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

impl Table {
    pub const fn new() -> Self {
        Self {
            slots: [None; CAPACITY],
            next: 1,
            cursor: CAPACITY - 1,
        }
    }
    pub fn create(&mut self) -> Result<(usize, Pid), Error> {
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Full)?;
        let next = self.next.checked_add(1).ok_or(Error::Exhausted)?;
        let pid = Pid(self.next);
        self.next = next;
        self.slots[slot] = Some(Record {
            pid,
            state: State::Ready,
        });
        Ok((slot, pid))
    }
    pub fn slot(&self, pid: Pid) -> Result<usize, Error> {
        self.slots
            .iter()
            .position(|r| r.is_some_and(|r| r.pid == pid))
            .ok_or(Error::Unknown)
    }
    pub fn state(&self, pid: Pid) -> Result<State, Error> {
        Ok(self.slots[self.slot(pid)?].unwrap().state)
    }
    pub fn schedule(&mut self) -> Result<Option<(usize, Pid)>, Error> {
        if self
            .slots
            .iter()
            .flatten()
            .any(|r| r.state == State::Running)
        {
            return Err(Error::Running);
        }
        for distance in 1..=CAPACITY {
            let slot = (self.cursor + distance) % CAPACITY;
            if let Some(record) = &mut self.slots[slot]
                && record.state == State::Ready
            {
                record.state = State::Running;
                self.cursor = slot;
                return Ok(Some((slot, record.pid)));
            }
        }
        Ok(None)
    }
    pub fn suspend(&mut self, pid: Pid) -> Result<(), Error> {
        let record = self.slots[self.slot(pid)?].as_mut().unwrap();
        if record.state != State::Running {
            return Err(Error::NotRunning);
        }
        record.state = State::Ready;
        Ok(())
    }
    pub fn finish(&mut self, pid: Pid, exit: Exit) -> Result<(), Error> {
        let record = self.slots[self.slot(pid)?].as_mut().unwrap();
        if matches!(record.state, State::Exited(_)) {
            return Err(Error::AlreadyExited);
        }
        record.state = State::Exited(exit);
        Ok(())
    }
    pub fn reap(&mut self, pid: Pid) -> Result<Exit, Error> {
        let slot = self.slot(pid)?;
        let State::Exited(exit) = self.slots[slot].unwrap().state else {
            return Err(Error::NotExited);
        };
        self.slots[slot] = None;
        Ok(exit)
    }
}
