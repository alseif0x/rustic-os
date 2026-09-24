// SPDX-License-Identifier: Apache-2.0
//! Diagnostic owner revocation during an in-flight V7 admission publication.
//!
//! The command arms the kernel's completion-hold diagnostic for the file
//! service, sends ACCEPT or EXECUTE without waiting, waits until the service
//! has a publication command held, and has the owner revoke this shell's file
//! binding through the supervisor (`REVOKE_SHELL_V7`). The service stops the
//! publication if its header was not submitted and acknowledges the
//! revocation only after settlement, so the job's completion means the
//! outcome is durable. The command then reports what the old endpoint
//! observed, adopts the new binding and prints the outcome read on it.
use super::*;
use crate::commands::operations::{pattern_byte, replacement};
use rustic_sdk::abi::{files::Packet, supervisor as sv};
use rustic_sdk::files::admission::AdmissionId;

/// `execute-admission-v7 ID revoke SKIP TICKS`
pub(super) fn execute(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
    exact(a, 5)?;
    if argument(a, 2)? != "revoke" {
        return Err(Error::Usage);
    }
    let id: AdmissionId = argument(a, 1)?.parse()?;
    let request = id.packet(rustic_sdk::abi::files::admission::EXECUTE, 0)?;
    revoke_during(s, request, number(a, 3)?, number(a, 4)?)?;
    print(s.files.admission_get(id)?);
    Ok(())
}

/// `admit-pattern-v7 WORKSPACE RESOURCE VERSION EPOCH KEY SEED SIZE revoke SKIP TICKS`
pub(super) fn accept(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
    exact(a, 11)?;
    if argument(a, 8)? != "revoke" {
        return Err(Error::Usage);
    }
    let request = replacement(a)?;
    let seed = u8::try_from(number(a, 6)?).map_err(|_| Error::Usage)?;
    let size = u32::try_from(number(a, 7)?).map_err(|_| Error::Usage)?;
    let fill = move |offset: u32, buffer: &mut [u8]| {
        for (index, byte) in (offset..).zip(buffer.iter_mut()) {
            *byte = pattern_byte(seed, index);
        }
        Ok(())
    };
    // Every chunk (and so every stage write) precedes the armed hold.
    let mut transfer = s.files.workspace_admission_open(request, size)?;
    while transfer.offset() < transfer.size() {
        if let Err(error) = s.files.workspace_chunk(&mut transfer, fill) {
            let _ = s.files.workspace_abort(transfer);
            return Err(error.into());
        }
    }
    let mut accept = Packet::new(rustic_sdk::abi::files::admission::ACCEPT);
    accept.id = request.resource.object();
    revoke_during(s, accept, number(a, 9)?, number(a, 10)?)?;
    print(s.files.admission_retry7(request.workspace, request.retry)?);
    Ok(())
}

/// Send `request`, hold the file service's publication command after `skip`
/// earlier mutations for up to `ticks`, revoke this shell's binding while it
/// is held, then adopt the new binding.
fn revoke_during(s: &mut Session, request: Packet, skip: u64, ticks: u64) -> Result<(), Error> {
    use rustic_sdk::rpc::Progress;
    s.service([sv::HOLD_IO, skip, ticks, 0, 0, 0, 0, 0])?;
    let started = rustic_sdk::runtime::clock();
    s.files.submit(request)?;
    // The service holds the reply until settlement, so the command it has
    // submitted is the publication's.
    let deadline = started.saturating_add(ticks);
    let held = loop {
        let status = s.request([sv::IO_STATUS, 0, 0, 0, 0, 0, 0, 0])?;
        if status[1] == 1 {
            break true;
        }
        if rustic_sdk::runtime::clock() >= deadline {
            break false;
        }
        s.files.progress().wait(0).map_err(|_| Error::Service(4))?;
    };
    let job = match s.request([sv::REVOKE_SHELL_V7, 0, 0, 0, 0, 0, 0, 0])? {
        reply if reply[0] == 5 => reply[1],
        _ => return Err(Error::Service(4)),
    };
    let status = s.wait_status(job)?;
    let ticks = rustic_sdk::runtime::clock().saturating_sub(started);
    if status[3] != 0 {
        return Err(Error::Service(status[3]));
    }
    // The old endpoint was closed with the reply outstanding.
    let old = s.files.poll();
    s.finish_job(status)?;
    output::format(format_args!(
        "revoke-v7 held={} job={job} old={} ticks={ticks}\r\n",
        u8::from(held),
        Old(old)
    ));
    Ok(())
}

/// What the revoked endpoint returned for the outstanding request.
struct Old(Result<Option<Packet>, rustic_sdk::files::Error>);
impl core::fmt::Display for Old {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Ok(Some(_)) => f.write_str("reply"),
            Ok(None) => f.write_str("pending"),
            Err(error) => write!(f, "{error:?}"),
        }
    }
}
