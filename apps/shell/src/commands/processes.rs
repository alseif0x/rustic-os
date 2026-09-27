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
                "lost-reply" | "lost-operation" | "lost-admission" => {
                    exact(a, 4)?;
                    (
                        if a.get(1) == Some("lost-admission") {
                            p::LOST_ADMISSION
                        } else if a.get(1) == Some("lost-operation") {
                            p::LOST_OPERATION
                        } else {
                            p::LOST_REPLY
                        },
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
            if !(2..=3).contains(&a.len()) || argument(a, 1)? != "files" {
                return Err(Error::Usage);
            }
            if a.len() == 3 && argument(a, 2)? == "timed" {
                // Guest ticks for the whole restart job, which a V7 service
                // spends almost entirely mounting (verifying every payload).
                let started = rustic_sdk::runtime::clock();
                s.service([p::RESTART, 0, 0, 0, 0, 0, 0, 0])?;
                let ticks = rustic_sdk::runtime::clock().saturating_sub(started);
                output::text("files restarted; utility sessions revoked\r\n");
                output::format(format_args!("restart-files ticks={ticks}\r\n"));
            } else if a.len() == 3 {
                if argument(a, 2)? != "async" {
                    return Err(Error::Usage);
                }
                let r = s.request([p::RESTART, 0, 0, 0, 0, 0, 0, 0])?;
                output::format(format_args!("files restart requested job={}\r\n", r[1]));
            } else {
                s.service([p::RESTART, 0, 0, 0, 0, 0, 0, 0])?;
                output::text("files restarted; utility sessions revoked\r\n");
            }
        }
        "ps" => {
            exact(a, 1)?;
            output::text("PID STATE EXIT CODE PREEMPTIONS PARENT PROGRAM\r\n");
            let slots = s.service([p::INFO, 0, 0, 0, 0, 0, 0, 0])?[3];
            for slot in 0..slots {
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
                            4 => "tasks",
                            5 => "staged",
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
                super::takeover::display(r);
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
                "files pid={} {}; files_source={}; shell pid={}; owner-policy id={} (0 means invalid; helper grants disabled)\r\n",
                r[1],
                if r[1] == 0 {
                    "unavailable"
                } else if r[4] == 0 {
                    "control-pending"
                } else {
                    "mounted"
                },
                if r[5] == 1 { "storage" } else { "embedded" },
                r[2],
                r[3]
            ));
        }
        "mem" => {
            exact(a, 1)?;
            let r = s.service([p::INFO, 0, 0, 0, 0, 0, 0, 0])?;
            output::format(format_args!(
                "ticks={} free_frames={} process_slots={} processes={} channels={} pending_io={} heap_pages={}\r\n",
                r[1], r[2], r[3], r[4], r[5], r[6], r[7]
            ));
        }
        "limits" => {
            exact(a, 1)?;
            let r = s.service([p::LIMITS, 0, 0, 0, 0, 0, 0, 0])?;
            let (processes, process_limit) = used_limit(r[1]);
            let (channels, channel_limit) = used_limit(r[2]);
            let (handles, handle_limit) = used_limit(r[3]);
            let (owner_handles, owner_handle_limit) = used_limit(r[4]);
            let (file_clients, file_client_limit) = used_limit(r[5]);
            let (child_slots, child_limit) = used_limit(r[6]);
            let process_reserve = r[7] as u16;
            let channel_reserve = (r[7] >> 16) as u16;
            let handle_reserve = (r[7] >> 32) as u16;
            let owner_reserve = (r[7] >> 48) as u16;
            output::format(format_args!(
                "processes used={processes}/{process_limit} available={} reserve={process_reserve}\r\n",
                process_limit
                    .saturating_sub(processes)
                    .saturating_sub(process_reserve)
            ));
            output::format(format_args!(
                "channels used={channels}/{channel_limit} available={} reserve={channel_reserve}\r\n",
                channel_limit
                    .saturating_sub(channels)
                    .saturating_sub(channel_reserve)
            ));
            output::format(format_args!(
                "endpoint_handles used={handles}/{handle_limit} available={} reserve={handle_reserve}\r\n",
                handle_limit
                    .saturating_sub(handles)
                    .saturating_sub(handle_reserve)
            ));
            output::format(format_args!(
                "owner_handles max={owner_handles}/{owner_handle_limit} available={} reserve={owner_reserve}\r\n",
                owner_handle_limit
                    .saturating_sub(owner_handles)
                    .saturating_sub(owner_reserve)
            ));
            let file_app_used = file_clients.saturating_sub(2);
            output::format(format_args!(
                "file_clients used={file_clients}/{file_client_limit} available={} reserved=2\r\n",
                file_client_limit
                    .saturating_sub(2)
                    .saturating_sub(file_app_used)
            ));
            output::format(format_args!(
                "child_slots used={child_slots}/{child_limit} available={}\r\n",
                child_limit.saturating_sub(child_slots)
            ));
        }
        _ => return Err(Error::Unknown),
    }
    Ok(())
}

fn used_limit(word: u64) -> (u16, u16) {
    (word as u16, (word >> 32) as u16)
}
