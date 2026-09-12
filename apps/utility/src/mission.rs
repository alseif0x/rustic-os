// SPDX-License-Identifier: Apache-2.0
//! One owner-stepped mission on one native Client. No parent-supplied version,
//! epoch or admission; the client reads and prepares its own scoped candidate.
use rustic_sdk::{
    abi::{services::Method, supervisor::actor as a},
    files::{
        Client, Error,
        operation::{Key, Replacement, Retry},
        read::{Info, Request},
    },
};

const CONTENT: &[u8] = b"Single native client";
#[derive(Default)]
pub(super) struct State {
    // Retain the retry tuple even if admission acknowledgement is uncertain.
    // This fixture never automatically retries or starts a second candidate.
    attempted: Option<Replacement>,
}

impl State {
    pub(super) fn execute(&mut self, action: u64, files: &mut Client, scope: u32) -> [u64; 8] {
        let result = match action {
            a::SELECT_GET | a::SELECT_CANCEL => {
                let method = if action == a::SELECT_GET {
                    Method::OperationsGet
                } else {
                    Method::OperationsCancel
                };
                files
                    .select_lifecycle(method)
                    .map(|d| [0, d.availability as u64, method as u64, 1, 2, 0, 0, 0])
            }
            a::MISSION_PREPARE => self.prepare(files, scope),
            a::MISSION_VERIFY => self.verify(files, scope),
            _ => Err(Error::Invalid),
        };
        result.unwrap_or_else(|e| [e as u64, 0, 0, 0, 0, 0, 0, 0])
    }

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

    fn prepare(&mut self, files: &mut Client, scope: u32) -> Result<[u64; 8], Error> {
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

    fn verify(&self, files: &mut Client, scope: u32) -> Result<[u64; 8], Error> {
        let request = self.attempted.ok_or(Error::Invalid)?;
        let mut bytes = [0; 1024];
        let info = Self::read(files, scope, &mut bytes)?;
        if &bytes[..info.length] != CONTENT
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
