// SPDX-License-Identifier: Apache-2.0
mod admission_actors;
mod admissions;
mod authority;
mod discovery;
mod files;
mod help;
mod lifecycle;
mod management;
mod negotiation;
mod operations;
mod processes;
mod read;
mod recovery;
mod takeover;
mod tasks;
use super::{output, session::Session};
use rustic_shell::parser::Args;
#[derive(Debug)]
pub enum Error {
    Usage,
    Unknown,
    File(rustic_sdk::files::Error),
    Service(u64),
    Pending(u64),
    TaskDocument,
    TaskCapacity,
    TaskEnable,
    TaskJournal,
    TaskPending(u64),
}
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Usage => f.write_str("invalid arguments; type help"),
            Self::Unknown => f.write_str("unknown command; type help"),
            Self::File(e) => write!(f, "{e:?}"),
            Self::TaskDocument => f.write_str("invalid tasks document"),
            Self::TaskCapacity => f.write_str("tasks capacity exceeded"),
            Self::TaskEnable => f.write_str("task writes require explicit setup: tasks enable"),
            Self::TaskJournal => {
                f.write_str("invalid or changed /config/tasks-intent; target not resubmitted")
            }
            Self::TaskPending(key) => {
                write!(f, "task intent={key} remains unresolved; use tasks recover")
            }
            Self::Pending(id) => write!(
                f,
                "wait interrupted; job={id} remains queryable with job-status {id}"
            ),
            Self::Service(code) => write!(
                f,
                "service {}",
                match code {
                    1 => "invalid request",
                    2 => "denied",
                    3 => "busy or full",
                    6 => "superseded; earlier submitted effects may still exist",
                    _ => "unavailable",
                }
            ),
        }
    }
}
impl From<rustic_sdk::files::Error> for Error {
    fn from(e: rustic_sdk::files::Error) -> Self {
        Self::File(e)
    }
}
impl From<rustic_tasks_client::Error> for Error {
    fn from(e: rustic_tasks_client::Error) -> Self {
        use rustic_tasks_client::Error as Task;
        match e {
            Task::File(e) => Self::File(e),
            Task::Service(code) => Self::Service(code),
            Task::Document => Self::TaskDocument,
            Task::Capacity => Self::TaskCapacity,
            Task::Enable => Self::TaskEnable,
            Task::Journal => Self::TaskJournal,
            Task::Pending(key) => Self::TaskPending(key),
        }
    }
}
pub fn argument<'a>(args: &Args<'a>, n: usize) -> Result<&'a str, Error> {
    args.get(n).ok_or(Error::Usage)
}
pub fn number(args: &Args<'_>, n: usize) -> Result<u64, Error> {
    argument(args, n)?.parse().map_err(|_| Error::Usage)
}
pub fn exact(args: &Args<'_>, n: usize) -> Result<(), Error> {
    if args.len() == n {
        Ok(())
    } else {
        Err(Error::Usage)
    }
}
pub fn execute(s: &mut Session, a: &Args<'_>) -> Result<bool, Error> {
    match argument(a, 0)? {
        "help" => help::execute(a)?,
        "tasks" => tasks::execute(s, a)?,
        #[cfg(feature = "tasks-acceptance")]
        "tasks-write-acceptance" => tasks::write_acceptance(s, a)?,
        #[cfg(feature = "tasks-acceptance")]
        "tasks-acceptance" => super::acceptance::execute(s, a)?,
        "echo" => {
            for i in 1..a.len() {
                if i > 1 {
                    output::text(" ");
                }
                output::bytes(argument(a, i)?.as_bytes());
            }
            output::text("\r\n");
        }
        "status" => {
            exact(a, 1)?;
            output::format(format_args!("{}\r\n", s.status));
        }
        "exit" => {
            exact(a, 1)?;
            output::text("Stopping RusticOS; committed files are on disk.\r\n");
            let _ = s.service([rustic_sdk::abi::supervisor::EXIT, 0, 0, 0, 0, 0, 0, 0]);
            return Ok(true);
        }
        "pwd" | "cd" | "ls" | "mkdir" | "touch" | "write" | "cat" | "stat" | "rm" => {
            files::execute(s, a)?
        }
        #[cfg(feature = "tasks-acceptance")]
        "tasks-owner-apply-cut" => authority::execute(s, a)?,
        "session" | "helper" | "act" | "actor-status" | "move-check" | "stall" | "revocation"
        | "tasks-owner" | "tasks-owner-begin" | "tasks-owner-edit" | "tasks-owner-chunk"
        | "tasks-owner-forget" => authority::execute(s, a)?,
        "retry-key" | "receipt" | "replace" | "rotate-receipts" => recovery::execute(s, a)?,
        "enable-admissions"
        | "enable-prevention-reasons"
        | "admit-ref"
        | "admission"
        | "execute-admission"
        | "cancel-admission"
        | "admission-activity"
        | "request-cancel"
        | "schedule-admission"
        | "observe-admission"
        | "observe-admission-v2" => admissions::execute(s, a)?,
        "admission-session" | "act-admission" => admission_actors::execute(s, a)?,
        "capabilities" => discovery::execute(s, a)?,
        "lifecycle-profile" | "select-lifecycle" => negotiation::execute(s, a)?,
        "inspect-selected" | "cancel-selected" => lifecycle::execute(s, a)?,
        "inspect-negotiated" | "cancel-negotiated" => lifecycle::execute(s, a)?,
        "inspect-operation" | "request-operation-cancel" => lifecycle::execute(s, a)?,
        "enable-operations" | "replace-ref" | "replace-fill-ref" | "operation" => {
            operations::execute(s, a)?
        }
        "ref" | "read-ref" => read::execute(s, a)?,
        "job-status" | "hold-io" | "io-status" => management::execute(s, a)?,
        "run" | "ps" | "kill" | "reap" | "permissions" | "revoke" | "services" | "mem"
        | "restart" => processes::execute(s, a)?,
        _ => return Err(Error::Unknown),
    }
    Ok(false)
}
