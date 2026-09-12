// SPDX-License-Identifier: Apache-2.0
mod admission_actors;
mod admissions;
mod authority;
mod files;
mod management;
mod operations;
mod processes;
mod read;
mod recovery;
mod takeover;
use super::{output, session::Session};
use rustic_shell::parser::Args;
#[derive(Debug)]
pub enum Error {
    Usage,
    Unknown,
    File(rustic_sdk::files::Error),
    Service(u64),
    Pending(u64),
}
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Usage => f.write_str("invalid arguments; type help"),
            Self::Unknown => f.write_str("unknown command; type help"),
            Self::File(e) => write!(f, "{e:?}"),
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
        "help" => {
            output::text(
                "admission-activity ADMISSION_ID | request-cancel ADMISSION_ID\r\nadmission-session FILE OTHER RIGHTS | act-admission PID execute|activity|request-cancel|lost-stop ADMISSION_ID\r\nLive cancellation replies acknowledge a request, not durable prevention.\r\n",
            );
            exact(a, 1)?;
            output::text(
                "help | pwd | cd PATH | ls [PATH] | mkdir PATH | touch PATH\r\nwrite PATH TEXT | cat PATH | stat PATH | rm PATH | echo TEXT | status\r\nrun spin|fault|exit | run read FILE | run probe FILE OTHER | run watch FILE [TICKS]\r\nps | kill PID | reap PID | permissions [PID] | revoke PID\r\nservices | mem | restart files | exit\r\nretry-key PATH KEY | replace PATH VERSION TOKEN TEXT | receipt ID TOKEN | rotate-receipts\r\nsession FILE OTHER [TICKS] | helper PID FILE OTHER | act PID read|stage|commit|flood|drain|stale\r\nmove-check CLIENT HELPER (moves its file endpoint)\r\nactor-status PID | revocation PID | stall files TICKS (0 = indefinite diagnostic)\r\njob-status [ID] | restart files [async] | hold-io SKIP TICKS | io-status\r\nref WORKSPACE PATH | read-ref WORKSPACE RESOURCE VERSION|- OFFSET LENGTH\r\nenable-operations | replace-ref WORKSPACE RESOURCE VERSION EPOCH KEY TEXT\r\nreplace-fill-ref WORKSPACE RESOURCE VERSION EPOCH KEY BYTE COUNT\r\noperation OPERATION_ID | operation WORKSPACE EPOCH KEY\r\nenable-admissions | admit-ref WORKSPACE RESOURCE VERSION EPOCH KEY TEXT\r\nadmission ADMISSION_ID | admission WORKSPACE EPOCH KEY\r\nexecute-admission ADMISSION_ID | cancel-admission ADMISSION_ID\r\nact PID api-read|read-open|read-next|fill (deterministic read diagnostics)\r\nCtrl-C interrupts a wait, not an already submitted effect.\r\nPaths: /system (read-only), /data, /config, /workspaces.\r\nLimits: 32 objects, 1024 bytes/file, 2 utility slots. No AI/network required.\r\n",
            );
        }
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
        "session" | "helper" | "act" | "actor-status" | "move-check" | "stall" | "revocation" => {
            authority::execute(s, a)?
        }
        "retry-key" | "receipt" | "replace" | "rotate-receipts" => recovery::execute(s, a)?,
        "enable-admissions" | "admit-ref" | "admission" | "execute-admission"
        | "cancel-admission" | "admission-activity" | "request-cancel" => {
            admissions::execute(s, a)?
        }
        "admission-session" | "act-admission" => admission_actors::execute(s, a)?,
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
