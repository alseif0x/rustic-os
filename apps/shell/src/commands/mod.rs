// SPDX-License-Identifier: Apache-2.0
mod files;
mod processes;
mod recovery;
use super::{output, session::Session};
use rustic_shell::parser::Args;
#[derive(Debug)]
pub enum Error {
    Usage,
    Unknown,
    File(rustic_sdk::files::Error),
    Service(u64),
}
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Usage => f.write_str("invalid arguments; type help"),
            Self::Unknown => f.write_str("unknown command; type help"),
            Self::File(e) => write!(f, "{e:?}"),
            Self::Service(code) => write!(
                f,
                "service {}",
                match code {
                    1 => "invalid request",
                    2 => "denied",
                    3 => "busy or full",
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
            exact(a, 1)?;
            output::text(
                "help | pwd | cd PATH | ls [PATH] | mkdir PATH | touch PATH\r\nwrite PATH TEXT | cat PATH | stat PATH | rm PATH | echo TEXT | status\r\nrun spin|fault|exit | run read FILE | run probe FILE OTHER | run watch FILE [TICKS]\r\nps | kill PID | reap PID | permissions [PID] | revoke PID\r\nservices | mem | restart files | exit\r\nretry-key PATH KEY | replace PATH VERSION TOKEN TEXT | receipt ID TOKEN | rotate-receipts\r\nPaths: /system (read-only), /data, /config, /workspaces.\r\nLimits: 32 objects, 1024 bytes/file, 2 utility slots. No AI/network required.\r\n",
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
        "retry-key" | "receipt" | "replace" | "rotate-receipts" => recovery::execute(s, a)?,
        "run" | "ps" | "kill" | "reap" | "permissions" | "revoke" | "services" | "mem"
        | "restart" => processes::execute(s, a)?,
        _ => return Err(Error::Unknown),
    }
    Ok(false)
}
