// SPDX-License-Identifier: Apache-2.0
use super::*;
use rustic_sdk::abi::supervisor as p;
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
