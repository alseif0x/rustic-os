// SPDX-License-Identifier: Apache-2.0
//! Cancellation while the real file-service grant is in flight.

use super::support;
use crate::commands::Error;
use crate::output;
use crate::session::Session;
use rustic_sdk::abi::supervisor;

pub(super) fn run(session: &mut Session, scope: u32) -> Result<(), Error> {
    support::reset(session)?;
    let before = support::memory(session)?;
    support::arm(session)?;
    let job = support::start_task(session, scope)?;
    let held = support::wait_held(session)?;
    let process = support::process_by_pid(session, held.pid)?;
    if process[2] != 1 || process[7] != 4 {
        return Err(Error::Service(4));
    }
    let aborted = session.request([supervisor::TASKS_ABORT, job, 0, 0, 0, 0, 0, 0])? == [0; 8];
    if !aborted {
        return Err(Error::Service(4));
    }
    if session.request([supervisor::JOB_STATUS, job, 0, 0, 0, 0, 0, 0])?
        != [0, job, supervisor::TASKS_LIST, 6, 0, 0, 0, 0]
    {
        return Err(Error::Service(4));
    }
    let drained = support::wait_drained(session, held.drained)?;
    if drained.admin_pending != 0 {
        return Err(Error::Service(4));
    }
    let stale_denied = matches!(
        session.request([supervisor::TASKS_ROW, job, 0, 0, 0, 0, 0, 0,]),
        Err(Error::Service(2))
    );
    if !stale_denied {
        return Err(Error::Service(4));
    }
    support::release(session)?;
    support::reset(session)?;
    let fresh_job = support::start_task(session, scope)?;
    let fresh = support::wait_task(session, fresh_job)?;
    let rows = support::read_all(session, fresh, scope)?;
    let after = support::memory(session)?;
    if before.frames != after.frames
        || before.processes != after.processes
        || before.channels != after.channels
        || before.pending != after.pending
        // The aborted listing child is reaped before this point, so its pages
        // are gone from the aggregate; equality, not zero, is what proves it.
        || before.heap != after.heap
    {
        return Err(Error::Service(4));
    }
    output::format(format_args!(
        "RUSTIC TASKS_LIFECYCLE case=cancel held={} dormant={} adminpending={} aborted={} drained={} stale_denied={} released=1 reset=1 fresh={} job={} task_pid={} slot={} before_frames={} after_frames={} before_processes={} after_processes={} before_channels={} after_channels={} before_pending={} after_pending={}\r\n",
        held.held,
        u64::from(process[2] == 1),
        held.admin_pending,
        u64::from(aborted),
        u64::from(drained.drained > held.drained),
        u64::from(stale_denied),
        u64::from(rows == support::EXPECTED_ROWS.len()),
        job,
        held.pid,
        held.slot,
        before.frames,
        after.frames,
        before.processes,
        after.processes,
        before.channels,
        after.channels,
        before.pending,
        after.pending,
    ));
    Ok(())
}
