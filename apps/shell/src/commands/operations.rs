// SPDX-License-Identifier: Apache-2.0
//! Manual entry points to the typed native operation API; no shell-owned mutation policy.
use super::*;
use rustic_sdk::abi::files::{
    operation::{Instance, Lookup, Operation, OperationId, Replacement, Retry},
    reference::{Epoch, Resource, Version, Workspace},
    workspace::{self, Operation as Operation7},
};
/// The printed fields of a completed-operation receipt in either profile.
struct Printed {
    id: OperationId,
    service_instance: Instance,
    workspace: Workspace,
    resource: Resource,
    previous_version: Version,
    version: Version,
    size: u32,
    retry: Retry,
    sha256: [u8; 32],
}
impl From<Operation> for Printed {
    fn from(o: Operation) -> Self {
        Self {
            id: o.id,
            service_instance: o.service_instance,
            workspace: o.workspace,
            resource: o.resource,
            previous_version: o.previous_version,
            version: o.version,
            size: u32::from(o.size),
            retry: o.retry,
            sha256: o.sha256,
        }
    }
}
impl From<workspace::Operation> for Printed {
    fn from(o: workspace::Operation) -> Self {
        Self {
            id: o.id,
            service_instance: o.service_instance,
            workspace: o.workspace,
            resource: o.resource,
            previous_version: o.previous_version,
            version: o.version,
            size: o.size,
            retry: o.retry,
            sha256: o.sha256,
        }
    }
}
fn print(operation: impl Into<Printed>) {
    let operation = operation.into();
    output::format(format_args!(
        "operation-v1 id={} service_instance={} state=succeeded effect=committed cancel_requested=false\r\n",
        operation.id, operation.service_instance
    ));
    output::format(format_args!(
        "receipt workspace={} resource={} previous_version={} version={} size={} epoch={} key={} sha256=",
        operation.workspace,
        operation.resource,
        operation.previous_version,
        operation.version,
        operation.size,
        operation.retry.epoch,
        operation.retry.key
    ));
    for byte in operation.sha256 {
        output::format(format_args!("{byte:02x}"));
    }
    output::text("\r\n");
}
/// Deterministic test content a host harness can recompute: byte `i` of seed
/// `s` is `s*31 + 7*i + i/509` modulo 256, so distinct seeds differ everywhere.
pub(super) fn pattern_byte(seed: u8, index: u32) -> u8 {
    seed.wrapping_mul(31)
        .wrapping_add(index.wrapping_mul(7) as u8)
        .wrapping_add((index / 509) as u8)
}
pub(super) fn replacement(a: &Args<'_>) -> Result<Replacement, Error> {
    Ok(Replacement {
        workspace: argument(a, 1)?.parse()?,
        resource: argument(a, 2)?.parse()?,
        expected_version: argument(a, 3)?.parse()?,
        retry: Retry {
            epoch: argument(a, 4)?.parse()?,
            key: argument(a, 5)?.parse()?,
        },
    })
}
pub(super) fn execute(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
    match argument(a, 0)? {
        "enable-operations" => {
            exact(a, 1)?;
            let r = s.service([
                rustic_sdk::abi::supervisor::ENABLE_OPERATIONS,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ])?;
            rustic_sdk::files::Error::parse(r[1] as u8)?;
            output::text("Workspace operations enabled; persistent format v3.\r\n");
        }
        "operation" => {
            let lookup = if a.len() == 2 {
                Lookup::Id(argument(a, 1)?.parse::<OperationId>()?)
            } else {
                exact(a, 4)?;
                Lookup::Retry {
                    workspace: argument(a, 1)?.parse::<Workspace>()?,
                    retry: Retry {
                        epoch: argument(a, 2)?.parse::<Epoch>()?,
                        key: argument(a, 3)?.parse()?,
                    },
                }
            };
            print(s.files.operation_get(lookup)?);
        }
        "replace-ref" | "replace-fill-ref" => {
            let fill = argument(a, 0)? == "replace-fill-ref";
            exact(a, if fill { 8 } else { 7 })?;
            let request = replacement(a)?;
            if fill {
                let byte = u8::try_from(number(a, 6)?).map_err(|_| Error::Usage)?;
                let count = usize::try_from(number(a, 7)?).map_err(|_| Error::Usage)?;
                if count > 1024 {
                    return Err(rustic_sdk::files::Error::Size.into());
                }
                print(s.files.replace_file(request, &[byte; 1024][..count])?);
            } else {
                print(s.files.replace_file(request, argument(a, 6)?.as_bytes())?);
            }
        }
        "operation-v7" => {
            let query = if a.len() == 2 {
                Lookup::Id(argument(a, 1)?.parse::<OperationId>()?)
            } else {
                exact(a, 4)?;
                Lookup::Retry {
                    workspace: argument(a, 1)?.parse::<Workspace>()?,
                    retry: Retry {
                        epoch: argument(a, 2)?.parse::<Epoch>()?,
                        key: argument(a, 3)?.parse()?,
                    },
                }
            };
            let started = rustic_sdk::runtime::clock();
            let operation = s.files.workspace_operation(query)?;
            let ticks = rustic_sdk::runtime::clock().saturating_sub(started);
            print(operation);
            output::format(format_args!(
                "lookup-v7 size={} ticks={ticks}\r\n",
                operation.size
            ));
        }
        "replace-pattern-v7" => {
            if a.len() != 8 && a.len() != 10 {
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
            if a.len() == 10 {
                return match argument(a, 8)? {
                    "cut" => cut(s, request, size, number(a, 9)?, fill),
                    "hold" => hold(s, request, size, number(a, 9)?, fill),
                    "probe" => {
                        let started = rustic_sdk::runtime::clock();
                        let (operation, probed) =
                            super::transfer_probe::probe(s, request, size, number(a, 9)?, fill)?;
                        let ticks = rustic_sdk::runtime::clock().saturating_sub(started);
                        print(operation);
                        output::format(format_args!("write-v7 size={size} ticks={ticks}\r\n"));
                        output::format(format_args!(
                            "probe-v7 every={} probes={} max={} p50={} total={} free_min={} free_max={} heap_min={} heap_max={}\r\n",
                            probed.every,
                            probed.count,
                            probed.max,
                            probed.p50(),
                            probed.total,
                            probed.free_frames.0,
                            probed.free_frames.1,
                            probed.heap_pages.0,
                            probed.heap_pages.1
                        ));
                        Ok(())
                    }
                    _ => Err(Error::Usage),
                };
            }
            let started = rustic_sdk::runtime::clock();
            let (operation, commit_ticks) = timed_replace(s, request, size, fill)?;
            let ticks = rustic_sdk::runtime::clock().saturating_sub(started);
            print(operation);
            output::format(format_args!(
                "write-v7 size={size} ticks={ticks} commit_ticks={commit_ticks}\r\n"
            ));
        }
        _ => return Err(Error::Unknown),
    }
    Ok(())
}

/// The SDK's streamed tracked replacement, step by step, so the commit
/// exchange can be timed on its own: `commit_ticks` runs from sending
/// `REPLACE_COMMIT` until the receipt is received and verified. The service
/// publishes the generation inside that exchange and serves no other request
/// meanwhile, so this bounds how long the single-loop service is blocked.
fn timed_replace(
    s: &mut Session,
    request: Replacement,
    size: u32,
    fill: impl Fn(u32, &mut [u8]) -> Result<(), rustic_sdk::files::Error> + Copy,
) -> Result<(Operation7, u64), Error> {
    let mut transfer = s.files.workspace_open(request, size)?;
    while transfer.offset() < transfer.size() {
        if let Err(error) = s.files.workspace_chunk(&mut transfer, fill) {
            let _ = s.files.workspace_abort(transfer);
            return Err(error.into());
        }
    }
    let started = rustic_sdk::runtime::clock();
    let operation = s.files.workspace_commit(transfer)?;
    Ok((
        operation,
        rustic_sdk::runtime::clock().saturating_sub(started),
    ))
}

/// Diagnostic cut of a V7 tracked write: send `chunks` chunks, have the owner
/// revoke this shell's file binding through the supervisor, and report what
/// the interrupted transfer observes on the old endpoint and on the new
/// binding. It never commits: the harness checks that nothing was published.
fn cut(
    s: &mut Session,
    request: Replacement,
    size: u32,
    chunks: u64,
    fill: impl Fn(u32, &mut [u8]) -> Result<(), rustic_sdk::files::Error> + Copy,
) -> Result<(), Error> {
    use rustic_sdk::abi::{files::DATA, supervisor as sv};
    // The cut must leave at least one chunk unsent.
    if chunks == 0 || chunks >= u64::from(size.div_ceil(DATA as u32)) {
        return Err(Error::Usage);
    }
    let mut transfer = s.files.workspace_open(request, size)?;
    for _ in 0..chunks {
        if let Err(error) = s.files.workspace_chunk(&mut transfer, fill) {
            let _ = s.files.workspace_abort(transfer);
            return Err(error.into());
        }
    }
    let sent = transfer.offset();
    // Revocation latency: from the owner's request to the completed job.
    let started = rustic_sdk::runtime::clock();
    let job = match s.request([sv::REVOKE_SHELL_V7, 0, 0, 0, 0, 0, 0, 0]) {
        Ok(job) if job[0] == 5 => job[1],
        refused => {
            let _ = s.files.workspace_abort(transfer);
            return Err(refused.err().unwrap_or(Error::Service(4)));
        }
    };
    let status = s.wait_status(job)?;
    let ticks = rustic_sdk::runtime::clock().saturating_sub(started);
    if status[3] != 0 {
        // The job failed, but the revocation may still have happened before a
        // later step failed, so this abort is best effort: on a revoked
        // binding it is refused and the service has already dropped the stage.
        let _ = s.files.workspace_abort(transfer);
        return Err(Error::Service(status[3]));
    }
    // The next chunk still goes to the endpoint the service closed.
    let old = s.files.workspace_chunk(&mut transfer, fill);
    s.finish_job(status)?;
    // The same transfer on the adopted binding names nothing the service holds.
    let new = s.files.workspace_chunk(&mut transfer, fill);
    output::format(format_args!(
        "cut-v7 chunks={chunks} bytes={sent} job={job} old={} new={} ticks={ticks}\r\n",
        Outcome(old),
        Outcome(new)
    ));
    Ok(())
}

/// Diagnostic hold of a V7 transfer: send `chunks` chunks, have the owner ask
/// for retention maintenance while the transfer is open, then abort it. The
/// service must refuse the maintenance with `Busy` and change nothing; the
/// harness checks the image. It never commits.
fn hold(
    s: &mut Session,
    request: Replacement,
    size: u32,
    chunks: u64,
    fill: impl Fn(u32, &mut [u8]) -> Result<(), rustic_sdk::files::Error> + Copy,
) -> Result<(), Error> {
    use rustic_sdk::abi::files::DATA;
    // The hold must leave at least one chunk unsent.
    if chunks == 0 || chunks >= u64::from(size.div_ceil(DATA as u32)) {
        return Err(Error::Usage);
    }
    let mut transfer = s.files.workspace_open(request, size)?;
    for _ in 0..chunks {
        if let Err(error) = s.files.workspace_chunk(&mut transfer, fill) {
            let _ = s.files.workspace_abort(transfer);
            return Err(error.into());
        }
    }
    let sent = transfer.offset();
    let maintained = super::retention::maintain(s);
    let aborted = s.files.workspace_abort(transfer);
    let maintain = match maintained {
        Ok(_) => Ok(()),
        Err(Error::File(error)) => Err(error),
        Err(other) => return Err(other),
    };
    output::format(format_args!(
        "hold-v7 chunks={chunks} bytes={sent} maintain={} abort={}\r\n",
        Outcome(maintain),
        Outcome(aborted)
    ));
    Ok(())
}

/// A chunk result as the harness reads it: `ok` or the file error name.
struct Outcome(Result<(), rustic_sdk::files::Error>);
impl core::fmt::Display for Outcome {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Ok(()) => f.write_str("ok"),
            Err(error) => write!(f, "{error:?}"),
        }
    }
}
