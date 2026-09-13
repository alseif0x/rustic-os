// SPDX-License-Identifier: Apache-2.0
//! Task command syntax and presentation; document semantics belong to the app.
use super::{Error, Session, argument, exact, output};
use rustic_sdk::{abi::supervisor as s, files, rpc::Progress, runtime};
use rustic_shell::parser::Args;
use rustic_tasks_contract::{MAX_TASKS, State, wire};

pub(super) fn execute(session: &mut Session, args: &Args<'_>) -> Result<(), Error> {
    let preview = argument(args, 1)? == "preview";
    let (path, mut request) = if preview {
        exact(args, 5)?;
        let edit = match argument(args, 2)? {
            "add" => rustic_tasks_contract::preview::Edit::add(argument(args, 4)?.as_bytes())
                .ok_or(Error::Usage)?,
            "done" => {
                let value = argument(args, 4)?;
                let id = value.parse::<u32>().map_err(|_| Error::Usage)?;
                if id == 0 || value.starts_with('0') || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(Error::Usage);
                }
                rustic_tasks_contract::preview::Edit::Done { id }
            }
            _ => return Err(Error::Usage),
        };
        let mut words = [0; 8];
        words[0] = s::TASKS_PREVIEW;
        words[2..].copy_from_slice(&edit.words());
        (argument(args, 3)?, words)
    } else {
        exact(args, 3)?;
        if argument(args, 1)? != "list" {
            return Err(Error::Usage);
        }
        (argument(args, 2)?, [s::TASKS_LIST, 0, 0, 0, 0, 0, 0, 0])
    };
    request[1] = session.files.resolve(session.cwd, path)?.into();
    let started = session.request(request)?;
    if started[0] != 5 || started[1] == 0 || started[2] != s::TASKS_LIST {
        return Err(Error::Service(4));
    }
    let job = started[1];
    let result = list(session, job, preview);
    if result.is_err() {
        // This command is read-only. Cancel provisioning/results, including an
        // interrupted wait before the child PID became known to this frontend.
        let _ = session.request([s::TASKS_ABORT, job, 0, 0, 0, 0, 0, 0]);
    }
    result.map_err(task_error)
}

fn list(session: &mut Session, job: u64, preview: bool) -> Result<(), Error> {
    let deadline = runtime::clock().saturating_add(1100);
    let complete = loop {
        let result = session.request([s::JOB_STATUS, job, 0, 0, 0, 0, 0, 0])?;
        if result[1] != job || result[2] != s::TASKS_LIST || result[7] != 0 {
            return Err(Error::Service(4));
        }
        if result[0] == 0 {
            break session.finish_job(result)?;
        }
        if runtime::clock() >= deadline {
            return Err(Error::Service(4));
        }
        progress(session)?;
    };
    let count = usize::try_from(complete[2]).map_err(|_| Error::Service(4))?;
    if complete[1] == 0 || count > MAX_TASKS || complete[3..].iter().any(|v| *v != 0) {
        return Err(Error::Service(4));
    }
    let mut rows = [None; MAX_TASKS];
    for index in 0..count {
        progress(session)?;
        let words = session.request([s::TASKS_ROW, job, index as u64, 0, 0, 0, 0, 0])?;
        let Some(wire::Response::Row(row)) = wire::decode_response(words) else {
            return Err(Error::Service(4));
        };
        if rows[..index]
            .iter()
            .flatten()
            .any(|old: &wire::Row| old.id == row.id)
        {
            return Err(Error::Service(4));
        }
        rows[index] = Some(row);
    }
    // Complete and release the result before printing, so interrupted/malformed
    // transport cannot present a partial list as a successful command.
    progress(session)?;
    let end = session.request([s::TASKS_ROW, job, count as u64, 0, 0, 0, 0, 0])?;
    let summary = if preview {
        if end[0..3] != [0, 0, count as u64] {
            return Err(Error::Service(4));
        }
        Some(
            rustic_tasks_contract::preview::Summary::decode([
                end[2], end[3], end[4], end[5], end[6], end[7], 0,
            ])
            .ok_or(Error::Service(4))?,
        )
    } else {
        if end != [0, 0, count as u64, 0, 0, 0, 0, 0] {
            return Err(Error::Service(4));
        }
        None
    };
    for row in rows[..count].iter().flatten() {
        let state = match row.state {
            State::Open => "open",
            State::Done => "done",
        };
        output::format(format_args!("{} [{}] ", row.id, state));
        output::bytes(&row.title[..usize::from(row.title_len)]);
        output::text("\r\n");
    }
    output::format(format_args!("{count} tasks\r\n"));
    if let Some(summary) = summary {
        output::format(format_args!(
            "preview task={} changed={} source_version={}; not applied\r\n",
            summary.task_id,
            u8::from(summary.changed),
            summary.version
        ));
    }
    Ok(())
}

fn progress(session: &mut Session) -> Result<(), Error> {
    session.files.progress().wait(0).map_err(|error| {
        Error::File(if matches!(error, rustic_sdk::Error::Interrupted) {
            files::Error::Interrupted
        } else {
            files::Error::Protocol
        })
    })
}

fn task_error(error: Error) -> Error {
    match error {
        Error::Service(7) => Error::TaskDocument,
        Error::Service(8) => Error::TaskCapacity,
        Error::Service(code @ 33..=63) => {
            Error::File(files::Error::parse((code - 32) as u8).unwrap_err())
        }
        other => other,
    }
}
