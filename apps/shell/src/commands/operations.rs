// SPDX-License-Identifier: Apache-2.0
//! Manual entry points to the typed native operation API; no shell-owned mutation policy.
use super::*;
use rustic_sdk::abi::files::{
    operation::{Instance, Lookup, Operation, OperationId, Replacement, Retry},
    reference::{Epoch, Resource, Version, Workspace},
    workspace,
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
fn pattern_byte(seed: u8, index: u32) -> u8 {
    seed.wrapping_mul(31)
        .wrapping_add(index.wrapping_mul(7) as u8)
        .wrapping_add((index / 509) as u8)
}
fn replacement(a: &Args<'_>) -> Result<Replacement, Error> {
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
        "replace-pattern-v7" => {
            exact(a, 8)?;
            let request = replacement(a)?;
            let seed = u8::try_from(number(a, 6)?).map_err(|_| Error::Usage)?;
            let size = u32::try_from(number(a, 7)?).map_err(|_| Error::Usage)?;
            let started = rustic_sdk::runtime::clock();
            let operation = s.files.workspace_replace(request, size, |offset, buffer| {
                for (index, byte) in (offset..).zip(buffer.iter_mut()) {
                    *byte = pattern_byte(seed, index);
                }
                Ok(())
            })?;
            let ticks = rustic_sdk::runtime::clock().saturating_sub(started);
            print(operation);
            output::format(format_args!("write-v7 size={size} ticks={ticks}\r\n"));
        }
        _ => return Err(Error::Unknown),
    }
    Ok(())
}
