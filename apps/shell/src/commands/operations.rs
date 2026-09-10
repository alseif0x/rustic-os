// SPDX-License-Identifier: Apache-2.0
//! Manual entry points to the typed native operation API; no shell-owned mutation policy.
use super::*;
use rustic_sdk::abi::files::{
    operation::{Lookup, Operation, OperationId, Replacement, Retry},
    reference::{Epoch, Workspace},
};
fn print(operation: Operation) {
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
            let request = Replacement {
                workspace: argument(a, 1)?.parse()?,
                resource: argument(a, 2)?.parse()?,
                expected_version: argument(a, 3)?.parse()?,
                retry: Retry {
                    epoch: argument(a, 4)?.parse()?,
                    key: argument(a, 5)?.parse()?,
                },
            };
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
        _ => return Err(Error::Unknown),
    }
    Ok(())
}
