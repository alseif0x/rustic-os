// SPDX-License-Identifier: Apache-2.0
//! Bounded owner-side helpers shared by the three lifecycle cases.

use crate::commands::Error;
use crate::session::Session;
use rustic_sdk::{abi::supervisor, rpc::Progress, runtime};
use rustic_tasks_contract::{State, acceptance as contract, wire};

const POLL_TICKS: u64 = 1200;
const PENDING: u64 = 5;

#[derive(Clone, Copy)]
pub(super) struct Memory {
    pub(super) frames: u64,
    pub(super) processes: u64,
    pub(super) channels: u64,
    pub(super) pending: u64,
}

#[derive(Clone, Copy)]
pub(super) struct FixtureStatus {
    pub(super) held: u64,
    pub(super) armed: u64,
    pub(super) slot: u64,
    pub(super) pid: u64,
    pub(super) admin_pending: u64,
    pub(super) drained: u64,
    pub(super) expiry_tick: u64,
}

#[derive(Clone, Copy)]
pub(super) struct TaskJob {
    pub(super) id: u64,
    pub(super) pid: u64,
    pub(super) count: usize,
}

#[derive(Clone, Copy)]
pub(super) struct ExpectedRow {
    pub(super) id: u32,
    pub(super) state: State,
    pub(super) title: &'static [u8],
}

pub(super) const EXPECTED_ROWS: [ExpectedRow; 2] = [
    ExpectedRow {
        id: 7,
        state: State::Open,
        title: b"Review kernel",
    },
    ExpectedRow {
        id: 42,
        state: State::Done,
        title: b"Boot the OS",
    },
];

pub(super) fn memory(session: &mut Session) -> Result<Memory, Error> {
    let words = session.request([supervisor::INFO, 0, 0, 0, 0, 0, 0, 0])?;
    if words[7] != 0 || words[3] == 0 {
        return Err(Error::Service(4));
    }
    Ok(Memory {
        frames: words[2],
        processes: words[4],
        channels: words[5],
        pending: words[6],
    })
}

pub(super) fn fixture_status(session: &mut Session) -> Result<FixtureStatus, Error> {
    let words = session.request(contract::request(contract::Request::Status))?;
    let Some(values) = contract::decode_status(words) else {
        return Err(Error::Service(4));
    };
    let slot = values[2].saturating_sub(1);
    Ok(FixtureStatus {
        held: values[0],
        armed: values[1],
        slot,
        pid: values[3],
        admin_pending: values[4],
        drained: values[5],
        expiry_tick: values[6],
    })
}

pub(super) fn arm(session: &mut Session) -> Result<(), Error> {
    control(session, contract::Request::Arm)
}

pub(super) fn release(session: &mut Session) -> Result<(), Error> {
    control(session, contract::Request::Release)
}

pub(super) fn reset(session: &mut Session) -> Result<(), Error> {
    control(session, contract::Request::Reset)
}

fn control(session: &mut Session, request: contract::Request) -> Result<(), Error> {
    if session.request(contract::request(request))? != [0; 8] {
        return Err(Error::Service(4));
    }
    Ok(())
}

pub(super) fn wait_held(session: &mut Session) -> Result<FixtureStatus, Error> {
    let deadline = runtime::clock().saturating_add(300);
    loop {
        let status = fixture_status(session)?;
        if status.held == 1 && status.armed == 1 && status.admin_pending == 1 && status.pid != 0 {
            return Ok(status);
        }
        if runtime::clock() >= deadline {
            return Err(Error::Service(4));
        }
        wait_owner_tick(session)?;
    }
}

pub(super) fn wait_drained(session: &mut Session, previous: u64) -> Result<FixtureStatus, Error> {
    let deadline = runtime::clock().saturating_add(300);
    loop {
        let status = fixture_status(session)?;
        if status.admin_pending == 0 && status.drained > previous {
            return Ok(status);
        }
        if runtime::clock() >= deadline {
            return Err(Error::Service(4));
        }
        wait_owner_tick(session)?;
    }
}

pub(super) fn start_task(session: &mut Session, scope: u32) -> Result<u64, Error> {
    let words = session.request([supervisor::TASKS_LIST, u64::from(scope), 0, 0, 0, 0, 0, 0])?;
    if words[0] != PENDING
        || words[1] == 0
        || words[2] != supervisor::TASKS_LIST
        || words[3] != 1
        || words[4..].iter().any(|word| *word != 0)
    {
        return Err(Error::Service(4));
    }
    Ok(words[1])
}

pub(super) fn wait_task(session: &mut Session, job: u64) -> Result<TaskJob, Error> {
    let deadline = runtime::clock().saturating_add(POLL_TICKS);
    loop {
        let words = session.request([supervisor::JOB_STATUS, job, 0, 0, 0, 0, 0, 0])?;
        if words[1] != job || words[2] != supervisor::TASKS_LIST || words[7] != 0 {
            return Err(Error::Service(4));
        }
        if words[0] == PENDING {
            if runtime::clock() >= deadline {
                return Err(Error::Service(4));
            }
            wait_owner_tick(session)?;
            continue;
        }
        if words[0] != 0 || words[3] != 0 {
            return Err(Error::Service(4));
        }
        let pid = words[4];
        let count = usize::try_from(words[5]).map_err(|_| Error::Service(4))?;
        if pid == 0 || count > rustic_tasks_contract::MAX_TASKS {
            return Err(Error::Service(4));
        }
        return Ok(TaskJob {
            id: job,
            pid,
            count,
        });
    }
}

pub(super) fn read_all(session: &mut Session, job: TaskJob, scope: u32) -> Result<usize, Error> {
    if job.count != EXPECTED_ROWS.len() {
        return Err(Error::Service(4));
    }
    let permissions = session.request([supervisor::PERMISSIONS, job.pid, 0, 0, 0, 0, 0, 0])?;
    if permissions[1] != u64::from(scope) || permissions[2] != 1 || permissions[3] == 0 {
        return Err(Error::Service(4));
    }
    for (index, expected) in EXPECTED_ROWS.iter().enumerate() {
        let words =
            session.request([supervisor::TASKS_ROW, job.id, index as u64, 0, 0, 0, 0, 0])?;
        verify_row(words, *expected)?;
    }
    let end = session.request([
        supervisor::TASKS_ROW,
        job.id,
        job.count as u64,
        0,
        0,
        0,
        0,
        0,
    ])?;
    if end != [0, 0, job.count as u64, 0, 0, 0, 0, 0] {
        return Err(Error::Service(4));
    }
    Ok(job.count)
}

pub(super) fn read_one(
    session: &mut Session,
    job: TaskJob,
    index: usize,
    expected: ExpectedRow,
) -> Result<(), Error> {
    verify_row(
        session.request([supervisor::TASKS_ROW, job.id, index as u64, 0, 0, 0, 0, 0])?,
        expected,
    )
}

fn verify_row(words: [u64; 8], expected: ExpectedRow) -> Result<(), Error> {
    let Some(wire::Response::Row(row)) = wire::decode_response(words) else {
        return Err(Error::Service(4));
    };
    let title_len = usize::from(row.title_len);
    if row.id != expected.id
        || row.state != expected.state
        || title_len != expected.title.len()
        || &row.title[..title_len] != expected.title
    {
        return Err(Error::Service(4));
    }
    Ok(())
}

pub(super) fn process_by_pid(session: &mut Session, pid: u64) -> Result<[u64; 8], Error> {
    for slot in 0..8 {
        let words = session.request([supervisor::PROCESS, slot, 0, 0, 0, 0, 0, 0])?;
        if words[1] == pid {
            return Ok(words);
        }
    }
    Err(Error::Service(4))
}

pub(super) fn wait_exited(session: &mut Session, pid: u64) -> Result<(), Error> {
    let deadline = runtime::clock().saturating_add(300);
    loop {
        let process = process_by_pid(session, pid)?;
        if process[2] == 5 {
            return Ok(());
        }
        if runtime::clock() >= deadline {
            return Err(Error::Service(4));
        }
        wait_owner_tick(session)?;
    }
}

pub(super) fn wait_runtime_only(session: &Session, deadline: u64) -> Result<(), Error> {
    while runtime::clock() < deadline {
        runtime::wait_set(&[session.supervisor.endpoint.token()], 100)
            .map_err(|_| Error::Service(4))?;
    }
    Ok(())
}

fn wait_owner_tick(session: &mut Session) -> Result<(), Error> {
    session
        .files
        .progress()
        .wait(0)
        .map_err(|_| Error::Service(4))
}
