// SPDX-License-Identifier: Apache-2.0
use super::*;
use rustic_sdk::abi::supervisor as p;
use rustic_shell::tasks_input;

pub(super) fn execute(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
    match argument(a, 0)? {
        "session" => {
            if !(3..=4).contains(&a.len()) {
                return Err(Error::Usage);
            }
            let id = s.files.resolve(s.cwd, argument(a, 1)?)?;
            let other = s.files.resolve(s.cwd, argument(a, 2)?)?;
            let lease = if a.len() == 4 { number(a, 3)? } else { 0 };
            let r = s.service([p::RUN, p::SESSION, id as u64, other as u64, 3, lease, 0, 0])?;
            output::format(format_args!("started pid={}\r\n", r[1]));
        }
        "tasks-owner" => {
            if !(3..=4).contains(&a.len()) {
                return Err(Error::Usage);
            }
            let id = s.files.resolve(s.cwd, argument(a, 1)?)?;
            let journal = s.files.resolve(s.cwd, argument(a, 2)?)?;
            // The two scopes are distinct objects by construction: a journal
            // aliasing the target would record its recovery evidence inside the
            // document that evidence is supposed to protect. The shell's own
            // record is refused for the same reason one client owns one record:
            // two clients sharing it would each treat the other's unresolved
            // intent as their own.
            if id == journal || super::tasks::record_object(s)? == Some(journal) {
                return Err(Error::Usage);
            }
            let lease = if a.len() == 4 { number(a, 3)? } else { 0 };
            let r = s.service([
                p::RUN,
                p::TASKS_OWNER,
                id as u64,
                journal as u64,
                7,
                lease,
                0,
                0,
            ])?;
            output::format(format_args!("started pid={}\r\n", r[1]));
        }
        "tasks-owner-begin" => {
            exact(a, 7)?;
            let r = s.service([
                p::TASKS_OWNER_BEGIN,
                number(a, 1)?,
                number(a, 2)?,
                number(a, 3)?,
                number(a, 4)?,
                number(a, 5)?,
                number(a, 6)?,
                0,
            ])?;
            super::takeover::actor(r);
        }
        "tasks-owner-edit" => {
            exact(a, 8)?;
            let r = s.service([
                p::TASKS_OWNER_EDIT,
                number(a, 1)?,
                number(a, 2)?,
                number(a, 3)?,
                number(a, 4)?,
                number(a, 5)?,
                number(a, 6)?,
                number(a, 7)?,
            ])?;
            super::takeover::actor(r);
        }
        "tasks-owner-chunk" => {
            exact(a, 3)?;
            let pid = number(a, 1)?;
            let c = tasks_input::chunk_words(argument(a, 2)?).ok_or(Error::Usage)?;
            let r = s.service([p::TASKS_OWNER_CHUNK, pid, c[0], c[1], c[2], c[3], c[4], 0])?;
            super::takeover::actor(r);
        }
        // An apply under an explicit failure cut; ordinary builds have no cut to
        // select, so they do not have the command either.
        #[cfg(feature = "tasks-acceptance")]
        "tasks-owner-apply-cut" => {
            exact(a, 3)?;
            let r = s.service([
                p::TASKS_OWNER_APPLY_CUT,
                number(a, 1)?,
                number(a, 2)?,
                0,
                0,
                0,
                0,
                0,
            ])?;
            super::takeover::actor(r);
        }
        "tasks-owner-forget" => {
            exact(a, 3)?;
            let r = s.service([
                p::TASKS_OWNER_FORGET,
                number(a, 1)?,
                number(a, 2)?,
                0,
                0,
                0,
                0,
                0,
            ])?;
            super::takeover::actor(r);
        }
        "helper" => {
            exact(a, 4)?;
            let parent = number(a, 1)?;
            // Reject locally disabled authority before consulting potentially stalled files.
            // The supervisor and service still enforce the actual derivation independently.
            if s.service([p::PERMISSIONS, parent, 0, 0, 0, 0, 0, 0])?[2] == 0 {
                return Err(Error::Service(2));
            }
            if s.service([p::SERVICES, 0, 0, 0, 0, 0, 0, 0])?[4] == 0 {
                return Err(Error::Service(3));
            }
            let id = s.files.resolve(s.cwd, argument(a, 2)?)?;
            let other = s.files.resolve(s.cwd, argument(a, 3)?)?;
            let r = s.service([p::HELPER_START, parent, id as u64, other as u64, 0, 0, 0, 0])?;
            output::format(format_args!("started pid={}\r\n", r[1]));
        }
        "act" | "move-check" => {
            exact(a, 3)?;
            let (op, value) = if argument(a, 0)? == "move-check" {
                (p::MOVE_CHECK, number(a, 2)?)
            } else {
                (
                    p::ACT,
                    match argument(a, 2)? {
                        "read" => p::actor::READ,
                        "stage" => p::actor::STAGE,
                        "commit" => p::actor::COMMIT,
                        "flood" => p::actor::FLOOD,
                        "drain" => p::actor::DRAIN,
                        "stale" => p::actor::STALE,
                        "api-read" => p::actor::API_READ,
                        "read-open" => p::actor::READ_OPEN,
                        "read-next" => p::actor::READ_NEXT,
                        "fill" => p::actor::FILL,
                        "operation-get" => p::actor::OPERATION_GET,
                        "capabilities" => p::actor::CAPABILITIES,
                        "profile-get" => p::actor::PROFILE_GET,
                        "profile-cancel" => p::actor::PROFILE_CANCEL,
                        "select-get" => p::actor::SELECT_GET,
                        "select-cancel" => p::actor::SELECT_CANCEL,
                        "mission-prepare" => p::actor::MISSION_PREPARE,
                        "mission-verify" => p::actor::MISSION_VERIFY,
                        "mission-schedule" => p::actor::MISSION_SCHEDULE,
                        "mission-inspect" => p::actor::MISSION_INSPECT,
                        "mission-cancel" => p::actor::MISSION_CANCEL,
                        "tasks-apply" => p::actor::TASKS_APPLY,
                        "tasks-status" => p::actor::TASKS_STATUS,
                        "tasks-recover" => p::actor::TASKS_RECOVER,
                        "tasks-heap-stress" => p::actor::TASKS_HEAP_STRESS,
                        _ => return Err(Error::Usage),
                    },
                )
            };
            let r = s.service([op, number(a, 1)?, value, 0, 0, 0, 0, 0])?;
            super::takeover::actor(r);
        }
        "actor-status" | "revocation" => {
            exact(a, 2)?;
            let op = if argument(a, 0)? == "actor-status" {
                p::ACT_STATUS
            } else {
                p::REVOCATION
            };
            let r = s.service([op, number(a, 1)?, 0, 0, 0, 0, 0, 0])?;
            if op == p::ACT_STATUS {
                super::takeover::actor(r)
            } else {
                super::takeover::display(r)
            }
        }
        "stall" => {
            exact(a, 3)?;
            if argument(a, 1)? != "files" {
                return Err(Error::Usage);
            }
            s.service([p::STALL_FILES, number(a, 2)?, 0, 0, 0, 0, 0, 0])?;
            output::text("files stall diagnostic armed; owner control remains available\r\n");
        }
        _ => return Err(Error::Unknown),
    }
    Ok(())
}
