// SPDX-License-Identifier: Apache-2.0
//! Abandoned-result expiry through the supervisor's runtime clock.

use super::support;
use crate::commands::Error;
use crate::output;
use crate::session::Session;
use rustic_sdk::{abi::supervisor, runtime};

const RESULT_TICKS: u64 = 1000;
const EXPIRY_MARGIN: u64 = 100;

pub(super) fn run(session: &mut Session, scope: u32) -> Result<(), Error> {
    support::reset(session)?;
    let before = support::memory(session)?;
    let start_tick = runtime::clock();
    let job = support::start_task(session, scope)?;
    let complete = support::wait_task(session, job)?;
    let complete_tick = runtime::clock();
    support::read_one(session, complete, 0, support::EXPECTED_ROWS[0])?;
    support::wait_runtime_only(
        session,
        complete_tick
            .saturating_add(RESULT_TICKS)
            .saturating_add(EXPIRY_MARGIN),
    )?;
    let query_sent_tick = runtime::clock();
    let status = support::fixture_status(session)?;
    let expired = u64::from(status.expiry_tick != 0 && status.expiry_tick < query_sent_tick);
    if expired == 0 {
        return Err(Error::Service(4));
    }
    let stale_denied = matches!(
        session.request([supervisor::TASKS_ROW, job, 1, 0, 0, 0, 0, 0,]),
        Err(Error::Service(2))
    );
    if !stale_denied {
        return Err(Error::Service(4));
    }
    let fresh_job = support::start_task(session, scope)?;
    let fresh = support::wait_task(session, fresh_job)?;
    let rows = support::read_all(session, fresh, scope)?;
    support::reset(session)?;
    let after = support::memory(session)?;
    if before.frames != after.frames
        || before.processes != after.processes
        || before.channels != after.channels
        || before.pending != after.pending
    {
        return Err(Error::Service(4));
    }
    output::format(format_args!(
        "RUSTIC TASKS_LIFECYCLE case=expiry completed=1 abandoned=1 expired={} stale_denied={} fresh={} rows={} start_tick={} complete_tick={} expiry_tick={} query_sent_tick={} before_frames={} after_frames={} before_processes={} after_processes={} before_channels={} after_channels={} before_pending={} after_pending={}\r\n",
        expired,
        u64::from(stale_denied),
        u64::from(rows == support::EXPECTED_ROWS.len()),
        rows,
        start_tick,
        complete_tick,
        status.expiry_tick,
        query_sent_tick,
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
