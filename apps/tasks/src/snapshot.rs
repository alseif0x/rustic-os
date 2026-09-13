// SPDX-License-Identifier: Apache-2.0
//! One complete, version-pinned source snapshot for list and edit preview.
use rustic_sdk::{
    abi::files::{
        read::{Info, Request},
        reference::Version,
    },
    files::{Client, Error},
};

pub(super) fn read(files: &mut Client, scope: u32, bytes: &mut [u8; 1024]) -> Result<Info, Error> {
    let metadata = files.stat(scope)?;
    if metadata.directory {
        return Err(Error::IsDirectory);
    }
    let references = files.references(u32::from(metadata.space), scope)?;
    let info = files.read_range(
        Request {
            workspace: references.workspace,
            resource: references.resource,
            expected_version: Some(Version::new(metadata.version)?),
            offset: 0,
            length: 1024,
        },
        bytes,
    )?;
    if !info.eof || info.offset != 0 || info.size as usize != info.length {
        return Err(Error::Protocol);
    }
    Ok(info)
}
