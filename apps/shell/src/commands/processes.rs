// SPDX-License-Identifier: Apache-2.0
use super::*;
use rustic_sdk::abi::supervisor as p;
pub(super) fn execute(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
    match argument(a, 0)? {
        "run" => {
            let (role, id, other, rights) = match argument(a, 1)? {
                "spin" | "fault" | "exit" => {
                    exact(a, 2)?;
                    (
                        match argument(a, 1)? {
                            "spin" => p::SPIN,
                            "fault" => p::FAULT,
                            _ => p::FINISH,
                        },
                        0,
                        0,
                        0,
                    )
                }
                "watch" => {
                    if !(3..=4).contains(&a.len()) {
                        return Err(Error::Usage);
                    }
                    (p::WATCH, s.files.resolve(s.cwd, argument(a, 2)?)?, 0, 1)
                }
                "read" => {
                    exact(a, 3)?;
                    (p::READ, s.files.resolve(s.cwd, argument(a, 2)?)?, 0, 1)
                }
                "lost-reply" => {
                    exact(a, 4)?;
                    (
                        p::LOST_REPLY,
                        s.files.resolve(s.cwd, argument(a, 2)?)?,
                        s.files.resolve(s.cwd, argument(a, 3)?)?,
                        7,
                    )
                }
                "probe" => {
                    exact(a, 4)?;
                    (
                        p::PROBE,
                        s.files.resolve(s.cwd, argument(a, 2)?)?,
                        s.files.resolve(s.cwd, argument(a, 3)?)?,
                        1,
                    )
                }
                _ => return Err(Error::Usage),
            };
            let r = s.service([
                p::RUN,
                role,
                id as u64,
                other as u64,
                rights,
                if a.get(1) == Some("watch") && a.len() == 4 {
                    number(a, 3)?
                } else {
                    0
                },
                0,
                0,
            ])?;
            output::format(format_args!("started pid={}\r\n", r[1]));
        }
        "restart" => {
            exact(a, 2)?;
            if argument(a, 1)? != "files" {
                return Err(Error::Usage);
            }
            let r = s.service([p::RESTART, 0, 0, 0, 0, 0, 0, 0])?;
            let old = core::mem::replace(
                &mut s.files,
                rustic_sdk::files::Client::new(r[2], r[1], r[3] as u32),
            );
            let _ = old.close();
            output::text("files restarted; utility sessions revoked\r\n");
        }
        "ps" => {
            exact(a, 1)?;
            output::text("PID STATE EXIT CODE PREEMPTIONS PARENT PROGRAM\r\n");
            for slot in 0..8 {
                let r = s.service([p::PROCESS, slot, 0, 0, 0, 0, 0, 0])?;
                if r[1] != 0 {
                    output::format(format_args!(
                        "{} {} {} {} {} {} {}\r\n",
                        r[1],
                        match r[2] {
                            1 => "dormant",
                            2 => "ready",
                            3 => "running",
                            4 => "blocked",
                            5 => "exited",
                            _ => "?",
                        },
                        r[3],
                        r[4],
                        r[5],
                        r[6],
                        match r[7] {
                            0 => "supervisor",
                            1 => "files",
                            2 => "shell",
                            3 => "utility",
                            _ => "?",
                        }
                    ));
                }
            }
        }
        "kill" | "reap" | "revoke" => {
            exact(a, 2)?;
            let op = match argument(a, 0)? {
                "kill" => p::KILL,
                "reap" => p::REAP,
                _ => p::REVOKE,
            };
            let r = s.service([op, number(a, 1)?, 0, 0, 0, 0, 0, 0])?;
            if op == p::REVOKE {
                output::format(format_args!(
                    "ok access=fenced members={} discarded_staging={} effects={} sequence={}\r\n",
                    r[1],
                    r[2],
                    if r[3] == 0 {
                        "settled"
                    } else {
                        "recovery-required"
                    },
                    r[4]
                ));
            } else {
                output::format(format_args!("ok exit_kind={} code={}\r\n", r[1], r[2]));
            }
        }
        "permissions" => {
            if a.len() > 2 {
                return Err(Error::Usage);
            }
            let pid = if a.len() == 2 { number(a, 1)? } else { 0 };
            let r = s.service([p::PERMISSIONS, pid, 0, 0, 0, 0, 0, 0])?;
            output::format(format_args!(
                "scope={} rights={} generation={} expires={} report={} bytes={} other={}\r\n",
                r[1], r[2], r[3], r[4], r[5], r[6], r[7]
            ));
        }
        "services" => {
            exact(a, 1)?;
            let r = s.service([p::SERVICES, 0, 0, 0, 0, 0, 0, 0])?;
            output::format(format_args!(
                "files pid={} {}; shell pid={}; owner-policy id={} (0 means invalid; helper grants disabled)\r\n",
                r[1],
                if r[1] == 0 { "unavailable" } else { "mounted" },
                r[2],
                r[3]
            ));
        }
        "mem" => {
            exact(a, 1)?;
            let r = s.service([p::INFO, 0, 0, 0, 0, 0, 0, 0])?;
            output::format(format_args!(
                "ticks={} free_frames={} process_slots={} processes={} channels={} pending_io={}\r\n",
                r[1], r[2], r[3], r[4], r[5], r[6]
            ));
        }
        _ => return Err(Error::Unknown),
    }
    Ok(())
}
