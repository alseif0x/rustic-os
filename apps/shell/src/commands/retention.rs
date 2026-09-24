// SPDX-License-Identifier: Apache-2.0
//! Owner retention maintenance of the V7 volume, started as a supervisor job.
//! The shell's file grant carries no maintenance authority: the request goes
//! over the shell's private owner channel, and the file service decides
//! whether it is safe now.
use super::*;
use rustic_sdk::abi::supervisor::{self as sv, maintenance};

/// A completed maintenance as the owner reads it.
pub(super) struct Maintained {
    pub job: u64,
    /// Newly published retry epoch; the previous one is `epoch - 1`.
    pub epoch: u64,
    pub records: u32,
    pub sectors: u32,
    /// Guest ticks from the request to the completed job.
    pub ticks: u64,
}

/// Start the owner's `MAINTAIN_V7` job and wait for it. A service refusal,
/// such as `Busy` while a transfer is open, is returned as that file error.
pub(super) fn maintain(s: &mut Session) -> Result<Maintained, Error> {
    let started = rustic_sdk::runtime::clock();
    let job = s.request([sv::MAINTAIN_V7, 0, 0, 0, 0, 0, 0, 0])?;
    if job[0] != 5 {
        return Err(Error::Service(4));
    }
    let r = s.wait_job(job[1])?;
    let ticks = rustic_sdk::runtime::clock().saturating_sub(started);
    rustic_sdk::files::Error::parse(u8::try_from(r[1]).map_err(|_| Error::Service(4))?)?;
    if r[2] < 2 {
        return Err(Error::Service(4));
    }
    let (records, sectors) = maintenance::split(r[3]);
    Ok(Maintained {
        job: job[1],
        epoch: r[2],
        records,
        sectors,
        ticks,
    })
}

pub(super) fn execute(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
    match argument(a, 0)? {
        "maintain-v7" => {
            exact(a, 1)?;
            let done = maintain(s)?;
            output::format(format_args!(
                "maintain-v7 previous=e_{:016x} epoch=e_{:016x} records={} sectors={} job={} ticks={}\r\n",
                done.epoch - 1,
                done.epoch,
                done.records,
                done.sectors,
                done.job,
                done.ticks
            ));
        }
        _ => return Err(Error::Unknown),
    }
    Ok(())
}
