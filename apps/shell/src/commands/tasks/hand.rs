// SPDX-License-Identifier: Apache-2.0
//! Hands one planned edit to a second native tasks client.
//!
//! The shell plans through the relay it already owns and then relays the plan,
//! one bounded step at a time, to a persistent tasks-owner child. It retains
//! nothing: its own recovery record is not read, written or created here, and
//! applying, recovering and forgetting stay with the child that owns the plan.
//! The supervisor refuses a step while the previous one is still pending, so
//! each step is polled to completion before the next one is sent.
use super::{Error, Session, argument, exact, output, parse_edit};
use rustic_sdk::{abi::supervisor as s, files, rpc::Progress};
use rustic_shell::parser::Args;
use rustic_tasks_client::plan;
use rustic_tasks_contract::candidate;

/// How many status queries one step may take before the shell stops waiting.
/// The supervisor gives a step its own deadline; this only bounds the shell.
const POLLS: usize = 400;

pub(super) fn execute(session: &mut Session, args: &Args<'_>) -> Result<(), Error> {
    exact(args, 6)?;
    let pid = argument(args, 2)?
        .parse::<u64>()
        .map_err(|_| Error::Usage)?;
    let edit = parse_edit(argument(args, 3)?, argument(args, 5)?)?;
    let scope = session.files.resolve(session.cwd, argument(args, 4)?)?;
    // Planning stays in the read-only native application. The shell only carries
    // what that application produced and what the plan says about itself.
    let candidate = plan(session, scope, edit)?;
    let summary = candidate.summary().words();
    let task = candidate.summary().task_id;
    let bytes = candidate.bytes();
    let edit = candidate.edit().words();
    step(
        session,
        pid,
        "edit",
        [
            s::TASKS_OWNER_EDIT,
            pid,
            edit[0],
            edit[1],
            edit[2],
            edit[3],
            edit[4],
            edit[5],
        ],
    )?;
    step(
        session,
        pid,
        "begin",
        [
            s::TASKS_OWNER_BEGIN,
            pid,
            bytes.len() as u64,
            summary[0],
            summary[1],
            summary[2],
            summary[3],
            0,
        ],
    )?;
    let mut chunks = 0;
    let mut offset = 0;
    // The child appends chunks in the order they arrive, so the order here is
    // the offset: the shell sends each chunk exactly once, in sequence.
    while offset < bytes.len() {
        let chunk = candidate::chunk(bytes, offset).ok_or(Error::TaskDocument)?;
        let words = candidate::owner_words(&chunk);
        step(
            session,
            pid,
            "chunk",
            [
                s::TASKS_OWNER_CHUNK,
                pid,
                words[3],
                words[4],
                words[5],
                words[6],
                words[7],
                0,
            ],
        )?;
        offset += chunk.length as usize;
        chunks += 1;
    }
    output::format(format_args!(
        "handed pid={pid} bytes={} chunks={chunks} task={task}\r\n",
        bytes.len()
    ));
    Ok(())
}

/// Delivers one step and waits for the child to answer it.
fn step(session: &mut Session, pid: u64, name: &str, request: [u64; 8]) -> Result<(), Error> {
    session.service(request)?;
    settle(session, pid, name)
}

/// Polls until the child's answer to the current step is complete, then reports
/// the child's own status word.
fn settle(session: &mut Session, pid: u64, name: &str) -> Result<(), Error> {
    for _ in 0..POLLS {
        let reply = session.service([s::ACT_STATUS, pid, 0, 0, 0, 0, 0, 0])?;
        match reply[1] {
            2 => {
                return match reply[2] {
                    0 => Ok(()),
                    code => Err(stopped(pid, name, Some(code))),
                };
            }
            1 => session
                .files
                .progress()
                .wait(0)
                .map_err(|_| Error::File(files::Error::Interrupted))?,
            // Idle means the step was never accepted; unconfirmed means its
            // deadline passed. Neither says what the child did with it.
            _ => return Err(stopped(pid, name, None)),
        }
    }
    Err(stopped(pid, name, None))
}

/// Stops the hand-off and says where it stopped.
///
/// Whatever the child already collected stays with the child; the next hand-off
/// announces a fresh plan, which always restarts its collection.
fn stopped(pid: u64, name: &str, code: Option<u64>) -> Error {
    match code {
        Some(code) => {
            output::format(format_args!(
                "hand-off stopped pid={pid} step={name} error={code}\r\n"
            ));
            child(code)
        }
        None => {
            output::format(format_args!(
                "hand-off stopped pid={pid} step={name} unconfirmed; query actor-status {pid}\r\n"
            ));
            Error::Service(4)
        }
    }
}

/// Reads one child refusal in the shell's own vocabulary.
///
/// The child encodes the owner client's refusals: the file ABI's numbering for
/// a file refusal, `64 + code` for a service refusal, and fixed codes for the
/// rest. A refusal of a hand-off step therefore reads exactly as the same
/// refusal would from the shell's own client. Codes this exchange cannot
/// produce are not reinterpreted; the printed line keeps the child's own code.
fn child(code: u64) -> Error {
    match code {
        1..=31 => match files::Error::parse(code as u8) {
            Err(error) => Error::File(error),
            Ok(()) => Error::Service(4),
        },
        65..=72 => Error::Service(code - 64),
        100 => Error::TaskDocument,
        101 => Error::TaskCapacity,
        102 => Error::TaskEnable,
        103 => Error::TaskJournal,
        _ => Error::Service(4),
    }
}
