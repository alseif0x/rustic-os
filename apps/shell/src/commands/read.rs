// SPDX-License-Identifier: Apache-2.0
//! Manual clients of the same bounded, versioned read used by native programs.
use super::*;
use rustic_sdk::abi::files::{
    read::Request,
    reference::{Resource, Version, Workspace},
};

pub(super) fn cat(s: &mut Session, id: u32) -> Result<(), Error> {
    let metadata = s.files.stat(id)?;
    if metadata.directory {
        return Err(rustic_sdk::files::Error::IsDirectory.into());
    }
    let mut bytes = [0; 1024];
    let length = match s.files.references(u32::from(metadata.space), id) {
        Ok(references) => {
            s.files
                .read_range(
                    Request {
                        workspace: references.workspace,
                        resource: references.resource,
                        expected_version: None,
                        offset: 0,
                        length: 1024,
                    },
                    &mut bytes,
                )?
                .length
        }
        // Legacy volumes retain manual access without inventing a stable identity
        // or silently upgrading their format. The explicit API stays unavailable.
        Err(rustic_sdk::files::Error::Unavailable) => s.files.read(id, &mut bytes)?,
        Err(error) => return Err(error.into()),
    };
    output::bytes(&bytes[..length]);
    output::text("\r\n");
    Ok(())
}

pub(super) fn execute(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
    if argument(a, 0)? == "ref" {
        exact(a, 3)?;
        let workspace = s.files.resolve(s.cwd, argument(a, 1)?)?;
        let resource = s.files.resolve(s.cwd, argument(a, 2)?)?;
        let references = s.files.references(workspace, resource)?;
        output::format(format_args!(
            "workspace={} resource={}\r\n",
            references.workspace, references.resource
        ));
        return Ok(());
    }
    exact(a, 6)?;
    let request = Request {
        workspace: argument(a, 1)?.parse::<Workspace>()?,
        resource: argument(a, 2)?.parse::<Resource>()?,
        expected_version: match argument(a, 3)? {
            "-" => None,
            version => Some(version.parse::<Version>()?),
        },
        offset: number(a, 4)?,
        length: number(a, 5)?.try_into().map_err(|_| Error::Usage)?,
    };
    let mut bytes = [0; 1024];
    let info = s.files.read_range(request, &mut bytes)?;
    output::format(format_args!(
        "read-v1 workspace={} resource={} version={} size={} offset={} length={} eof={} retry_epoch={} range_sha256=",
        request.workspace,
        request.resource,
        info.version,
        info.size,
        info.offset,
        info.length,
        info.eof,
        info.retry_epoch,
    ));
    for byte in info.range_sha256 {
        output::format(format_args!("{byte:02x}"));
    }
    output::text("\r\ndata=");
    for byte in &bytes[..info.length] {
        output::format(format_args!("{byte:02x}"));
    }
    output::text("\r\n");
    Ok(())
}
