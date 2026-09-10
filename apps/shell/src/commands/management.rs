// SPDX-License-Identifier: Apache-2.0
//! Owner operation results and explicit device diagnostics.
use super::*;
use rustic_sdk::abi::supervisor as p;
pub(super) fn execute(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
    match argument(a, 0)? {
        "job-status" => {
            if a.len() > 2 {
                return Err(Error::Usage);
            }
            let id = if a.len() == 2 { number(a, 1)? } else { 0 };
            let r = s.request([p::JOB_STATUS, id, 0, 0, 0, 0, 0, 0])?;
            if r[0] == 5 {
                output::format(format_args!(
                    "job={} pending kind={} phase={} service={} pending_io={}\r\n",
                    r[1], r[2], r[3], r[4], r[5]
                ));
            } else {
                output::format(format_args!(
                    "job={} complete kind={} status={} value={} token={} generation={}\r\n",
                    r[1], r[2], r[3], r[4], r[5], r[6]
                ));
                s.finish_job(r)?;
            }
        }
        "hold-io" => {
            exact(a, 3)?;
            s.service([p::HOLD_IO, number(a, 1)?, number(a, 2)?, 0, 0, 0, 0, 0])?;
            output::text("completion observation diagnostic armed\r\n");
        }
        "io-status" => {
            exact(a, 1)?;
            let r = s.service([p::IO_STATUS, 0, 0, 0, 0, 0, 0, 0])?;
            output::format(format_args!(
                "held={} owner={} request={} until={} armed={} skip={} ticks={}\r\n",
                r[1], r[2], r[3], r[4], r[5], r[6], r[7]
            ));
        }
        _ => return Err(Error::Unknown),
    }
    Ok(())
}
