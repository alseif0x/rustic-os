// SPDX-License-Identifier: Apache-2.0
//! A normal utility launched while the tasks child slot is reserved.

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
    let dormant = support::process_by_pid(session, held.pid)?;
    if dormant[2] != 1 || dormant[7] != 4 {
        return Err(Error::Service(4));
    }
    let spin = session.request([supervisor::RUN, supervisor::SPIN, 0, 0, 0, 0, 0, 0])?;
    if spin[0] != 0 || spin[1] == 0 {
        return Err(Error::Service(4));
    }
    let spin_pid = spin[1];
    let spin_process = support::process_by_pid(session, spin_pid)?;
    let spin_permissions =
        session.request([supervisor::PERMISSIONS, spin_pid, 0, 0, 0, 0, 0, 0])?;
    let ownership = u64::from(
        spin_pid != held.pid
            && spin_process[7] == 3
            && dormant[7] == 4
            && spin_permissions[1] == 0
            && spin_permissions[2] == 0,
    );
    let released = support::release(session).is_ok();
    if !released {
        return Err(Error::Service(4));
    }
    let complete = support::wait_task(session, job)?;
    if complete.pid != held.pid {
        return Err(Error::Service(4));
    }
    let task_permissions =
        session.request([supervisor::PERMISSIONS, complete.pid, 0, 0, 0, 0, 0, 0])?;
    let perms = u64::from(
        task_permissions[1] == u64::from(scope)
            && task_permissions[2] == 1
            && task_permissions[3] != 0,
    );
    let rows = support::read_all(session, complete, scope)?;
    let spin_permissions_after =
        session.request([supervisor::PERMISSIONS, spin_pid, 0, 0, 0, 0, 0, 0])?;
    let spin_owned_after_cleanup = u64::from(
        spin_permissions_after[1] == spin_permissions[1]
            && spin_permissions_after[2] == spin_permissions[2],
    );
    if session.request([supervisor::KILL, spin_pid, 0, 0, 0, 0, 0, 0])? != [0; 8] {
        return Err(Error::Service(4));
    }
    support::wait_exited(session, spin_pid)?;
    let reaped = session.request([supervisor::REAP, spin_pid, 0, 0, 0, 0, 0, 0])?;
    if reaped[0] != 0 || reaped[1] == 0 || reaped[3..].iter().any(|word| *word != 0) {
        return Err(Error::Service(4));
    }
    support::reset(session)?;
    let after = support::memory(session)?;
    if before.frames != after.frames
        || before.processes != after.processes
        || before.channels != after.channels
        || before.pending != after.pending
        // The killed spin process is explicitly reaped above, which is what
        // releases its heap pages: an unreaped exit would still be counted.
        || before.heap != after.heap
    {
        return Err(Error::Service(4));
    }
    output::format(format_args!(
        "RUSTIC TASKS_LIFECYCLE case=concurrent held={} dormant={} adminpending={} reserved={} spin_started=1 distinct_pids={} ownership={} perms={} released={} listed={} rows={} task_pid={} spin_pid={} job={} before_frames={} after_frames={} before_processes={} after_processes={} before_channels={} after_channels={} before_pending={} after_pending={}\r\n",
        held.held,
        u64::from(dormant[2] == 1),
        held.admin_pending,
        held.held,
        u64::from(spin_pid != held.pid),
        ownership,
        perms & spin_owned_after_cleanup,
        u64::from(released),
        u64::from(rows == support::EXPECTED_ROWS.len()),
        rows,
        complete.pid,
        spin_pid,
        job,
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
