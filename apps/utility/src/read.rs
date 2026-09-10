// SPDX-License-Identifier: Apache-2.0
//! Owner-stepped read diagnostics over real SDK and authenticated file packets.
use rustic_sdk::{
    abi::{
        files::{
            self as f,
            read::{Header, Info, Request},
        },
        supervisor::actor as a,
    },
    files::Client,
};

#[derive(Default)]
pub(super) struct State {
    pending: Option<(Request, Header)>,
}

impl State {
    fn request(files: &mut Client, scope: u32) -> Result<Request, f::Error> {
        let metadata = files.stat(scope)?;
        let references = files.references(u32::from(metadata.space), scope)?;
        Ok(Request {
            workspace: references.workspace,
            resource: references.resource,
            expected_version: None,
            offset: 0,
            length: 1024,
        })
    }

    pub(super) fn execute(
        &mut self,
        action: u64,
        files: &mut Client,
        scope: u32,
        other: u32,
    ) -> [u64; 8] {
        let result = match action {
            a::API_READ => Self::read(files, scope, other),
            a::READ_OPEN => self.open(files, scope),
            a::READ_NEXT => self.next(files),
            a::FILL => Self::fill(files, scope),
            _ => Err(f::Error::Invalid),
        };
        result.unwrap_or_else(|error| [error as u64, 0, 0, 0, 0, 0, 0, 0])
    }

    fn read(files: &mut Client, scope: u32, other: u32) -> Result<[u64; 8], f::Error> {
        let request = Self::request(files, scope)?;
        let mut bytes = [0; 1024];
        let info = files.read_range(request, &mut bytes)?;
        let other = files
            .references(request.workspace.root(), other)
            .err()
            .map_or(0, |e| e as u64);
        // An epoch returned to an ordinary reader must not grant receipt visibility.
        let denied = files.retry_token(scope, 1) == Err(f::Error::Denied);
        Ok([
            0,
            info.length as u64,
            other,
            u64::from(denied && info.retry_epoch.value() != 0),
            info.version.value(),
            0,
            0,
            0,
        ])
    }

    fn fill(files: &mut Client, scope: u32) -> Result<[u64; 8], f::Error> {
        let mut bytes = [0; 1024];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = (index.wrapping_mul(37).wrapping_add(11) % 256) as u8;
        }
        let version = files.stat(scope)?.version;
        let metadata = files.replace(scope, version, &bytes)?;
        Ok([0, 1024, 0, 0, metadata.version, 0, 0, 0])
    }

    fn open(&mut self, files: &mut Client, scope: u32) -> Result<[u64; 8], f::Error> {
        self.pending = None;
        let request = Self::request(files, scope)?;
        let header = Header::decode(&files.request(request.packet(f::READ_OPEN, files.context)?)?)?;
        Info::from_header(request, header)?;
        let first = Request {
            expected_version: Some(header.version),
            length: 40,
            ..request
        };
        let reply = files.request(first.packet(f::READ_CHUNK, files.context)?)?;
        Self::check(first, header, &reply)?;
        self.pending = Some((
            Request {
                offset: u64::from(reply.count),
                ..first
            },
            header,
        ));
        Ok([
            0,
            u64::from(reply.count),
            0,
            0,
            header.version.value(),
            0,
            0,
            0,
        ])
    }

    fn next(&mut self, files: &mut Client) -> Result<[u64; 8], f::Error> {
        let (request, header) = self.pending.take().ok_or(f::Error::NoTransfer)?;
        let reply = files.request(request.packet(f::READ_CHUNK, files.context)?)?;
        Self::check(request, header, &reply)?;
        Ok([
            0,
            u64::from(reply.count),
            0,
            0,
            header.version.value(),
            0,
            0,
            0,
        ])
    }

    fn check(request: Request, header: Header, reply: &f::Packet) -> Result<(), f::Error> {
        if reply.id != header.id
            || u64::from(reply.arg) != header.size
            || reply.version != header.version.value()
            || request.offset > header.size
            || u64::from(reply.count)
                != (header.size - request.offset).min(u64::from(request.length))
        {
            Err(f::Error::Protocol)
        } else {
            Ok(())
        }
    }
}
