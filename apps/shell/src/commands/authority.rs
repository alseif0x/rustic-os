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
            let id = s.files.resolve(s.cwd, argument(a, 2)?)?;
            let other = s.files.resolve(s.cwd, argument(a, 3)?)?;
            let r = s.service([
                p::HELPER_START,
                number(a, 1)?,
                id as u64,
                other as u64,
                0,
                0,
                0,
                0,
            ])?;
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
                        _ => return Err(Error::Usage),
                    },
                )
            };
            let r = s.service([op, number(a, 1)?, value, 0, 0, 0, 0, 0])?;
            output::format(format_args!(
                "actor status={} value={} other={} control_denied={} version={}\r\n",
                r[1], r[2], r[3], r[4], r[5]
            ));
        }
        _ => return Err(Error::Unknown),
    }
    Ok(())
}
