// SPDX-License-Identifier: Apache-2.0
//! Explicit owner provisioning and stepping of deterministic admission clients.
use super::*;
use rustic_sdk::abi::{files::admission as a, supervisor as s};
pub(super) fn execute(session: &mut Session, args: &Args<'_>) -> Result<(), Error> {
    if argument(args, 0)? == "admission-session" {
        let role = match args.len() {
            4 => s::ADMISSION_SESSION,
            5 if argument(args, 4)? == "private" => s::PRIVATE_ADMISSION_SESSION,
            _ => return Err(Error::Usage),
        };
        let scope = session.files.resolve(session.cwd, argument(args, 1)?)?;
        let other = session.files.resolve(session.cwd, argument(args, 2)?)?;
        let rights = number(args, 3)?;
        let r = session.service([s::RUN, role, scope as u64, other as u64, rights, 0, 0, 0])?;
        output::format(format_args!("started pid={}\r\n", r[1]));
    } else {
        exact(args, 4)?;
        let id: a::AdmissionId = argument(args, 3)?.parse()?;
        let lineage = id.lineage();
        let (op, discard) = match argument(args, 2)? {
            "execute" => (a::EXECUTE, 0),
            "schedule" => (a::SCHEDULE, 0),
            "lost-schedule" => (a::SCHEDULE, s::actor::flags::DISCARD_REPLY),
            "get" => (a::GET, 0),
            "lost-result" => (a::GET, s::actor::flags::DISCARD_REPLY),
            "activity" => (a::ACTIVITY, 0),
            "request-cancel" => (a::REQUEST_CANCEL, 0),
            "lost-stop" => (a::REQUEST_CANCEL, s::actor::flags::DISCARD_REPLY),
            _ => return Err(Error::Usage),
        };
        let r = session.service([
            s::ACT_ADMISSION,
            number(args, 1)?,
            u64::from_le_bytes(lineage[..8].try_into().unwrap()),
            u64::from_le_bytes(lineage[8..].try_into().unwrap()),
            id.number(),
            op as u64,
            discard,
            0,
        ])?;
        super::takeover::actor(r);
    }
    Ok(())
}
