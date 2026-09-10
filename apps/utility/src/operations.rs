// SPDX-License-Identifier: Apache-2.0
//! A read-capable helper may observe the epoch but cannot inspect durable operations.
use rustic_sdk::{
    abi::files::{
        Error,
        operation::{Key, Lookup, Retry},
        read::Request,
    },
    files::Client,
};
pub fn inspect(files: &mut Client, scope: u32) -> [u64; 8] {
    let result = (|| {
        let metadata = files.stat(scope)?;
        let refs = files.references(metadata.parent, scope)?;
        let mut byte = [0; 1];
        let info = files.read_range(
            Request {
                workspace: refs.workspace,
                resource: refs.resource,
                expected_version: None,
                offset: 0,
                length: 1,
            },
            &mut byte,
        )?;
        files.operation_get(Lookup::Retry {
            workspace: refs.workspace,
            retry: Retry {
                epoch: info.retry_epoch,
                key: Key::new(77)?,
            },
        })?;
        Ok::<_, Error>(())
    })();
    [result.err().map_or(0, |e| e as u64), 0, 0, 0, 0, 0, 0, 0]
}
