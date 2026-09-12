// SPDX-License-Identifier: Apache-2.0
//! Read, retain and admit one candidate with the version actually observed.
use super::State;
use rustic_sdk::files::{
    Client, Error,
    operation::{Key, Replacement, Retry},
    read::{Info, Request},
};
const CONTENT: &[u8] = b"Single native client";

impl State {
    fn read(files: &mut Client, scope: u32, bytes: &mut [u8; 1024]) -> Result<Info, Error> {
        let metadata = files.stat(scope)?;
        let refs = files.references(u32::from(metadata.space), scope)?;
        files.read_range(
            Request {
                workspace: refs.workspace,
                resource: refs.resource,
                expected_version: None,
                offset: 0,
                length: 1024,
            },
            bytes,
        )
    }

    pub(super) fn prepare(&mut self, files: &mut Client, scope: u32) -> Result<[u64; 8], Error> {
        if self.attempted.is_some() {
            return Err(Error::Busy);
        }
        let info = Self::read(files, scope, &mut [0; 1024])?;
        let request = Replacement {
            workspace: info.references.workspace,
            resource: info.references.resource,
            expected_version: info.version,
            retry: Retry {
                epoch: info.retry_epoch,
                key: Key::new(0x8300)?,
            },
        };
        self.attempted = Some(request);
        let status = files.admit_file(request, CONTENT)?;
        self.id = Some(status.id);
        Ok([
            0,
            status.id.number(),
            info.retry_epoch.value(),
            info.length as u64,
            info.version.value(),
            0,
            0,
            0,
        ])
    }

    pub(super) fn verify(&self, files: &mut Client, scope: u32) -> Result<[u64; 8], Error> {
        let request = self.attempted.ok_or(Error::Invalid)?;
        // Matching bytes alone do not establish that this mission committed.
        let operation = files.inspect_selected(self.id.ok_or(Error::Invalid)?)?;
        let rustic_sdk::files::lifecycle::State::Succeeded { completion_id } = operation.state
        else {
            return Err(Error::Invalid);
        };
        let mut bytes = [0; 1024];
        let info = Self::read(files, scope, &mut bytes)?;
        if &bytes[..info.length] != CONTENT
            || info.version.value() != completion_id.sequence()
            || info.version.value() <= request.expected_version.value()
        {
            return Err(Error::Protocol);
        }
        // read_range independently verifies the returned range SHA-256.
        Ok([
            0,
            info.length as u64,
            info.retry_epoch.value(),
            1,
            info.version.value(),
            0,
            0,
            0,
        ])
    }
}
