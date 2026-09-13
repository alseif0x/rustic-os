// SPDX-License-Identifier: Apache-2.0
//! Complete bounded native app results before presenting or retaining them.
use super::{Error, Session, output};
use rustic_sdk::{abi::supervisor as s, files, rpc::Progress, runtime};
use rustic_tasks_contract::{
    MAX_TASKS, State,
    preview::{Edit, Summary},
    wire,
};

pub(super) struct Candidate {
    pub bytes: [u8; 1024],
    pub length: usize,
    pub summary: Summary,
}

pub(super) fn run(
    session: &mut Session,
    scope: u32,
    edit: Option<Edit>,
    retain: bool,
) -> Result<Option<Candidate>, Error> {
    let mut request = [s::TASKS_LIST, scope.into(), 0, 0, 0, 0, 0, 0];
    if let Some(edit) = edit {
        request[0] = s::TASKS_PREVIEW;
        request[2..].copy_from_slice(&edit.words());
    }
    let started = session.request(request)?;
    if started[0] != 5 || started[1] == 0 || started[2] != s::TASKS_LIST {
        return Err(Error::Service(4));
    }
    let result = collect(session, started[1], edit.is_some(), retain);
    if result.is_err() {
        let _ = session.request([s::TASKS_ABORT, started[1], 0, 0, 0, 0, 0, 0]);
    }
    result.map_err(task_error)
}

fn collect(
    session: &mut Session,
    job: u64,
    preview: bool,
    retain: bool,
) -> Result<Option<Candidate>, Error> {
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
    let mut candidate_bytes = [0; 1024];
    let mut length: usize = 0;
    let mut total = None;
    if retain {
        loop {
            let words = session.request([s::TASKS_CANDIDATE, job, length as u64, 0, 0, 0, 0, 0])?;
            // Owner success framing carries the same canonical metadata/payload
            // as the child chunk. The codec validates bounds and zero padding.
            let part = rustic_tasks_contract::candidate::Chunk::decode_owner(words)
                .ok_or(Error::Service(4))?;
            if part.offset != length as u64 || total.is_some_and(|value| value != part.total) {
                return Err(Error::Service(4));
            }
            total = Some(part.total);
            let end = length + part.length as usize;
            candidate_bytes[length..end].copy_from_slice(&part.bytes[..part.length as usize]);
            length = end;
            if length as u64 == part.total {
                break;
            }
        }
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
    if retain {
        let summary = summary.ok_or(Error::Service(4))?;
        let document = rustic_tasks_contract::Document::parse(&candidate_bytes[..length])
            .map_err(|_| Error::Service(4))?;
        if document.len() != count {
            return Err(Error::Service(4));
        }
        for (index, row) in rows[..count].iter().flatten().enumerate() {
            let task = document.get(index).ok_or(Error::Service(4))?;
            if task.id != row.id
                || task.state != row.state
                || task.title() != &row.title[..row.title_len as usize]
            {
                return Err(Error::Service(4));
            }
        }
        return Ok(Some(Candidate {
            bytes: candidate_bytes,
            length,
            summary,
        }));
    }
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
    Ok(None)
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
