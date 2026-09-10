// SPDX-License-Identifier: Apache-2.0
use super::{MAX_RANGE, Request};
use crate::files::{
    Error, Packet, READ_OPEN,
    reference::{Epoch, References, Version},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub id: u32,
    pub size: u64,
    pub version: Version,
    pub range_sha256: [u8; 32],
    pub retry_epoch: Epoch,
}
impl Header {
    pub fn packet(self, context: u32) -> Result<Packet, Error> {
        if self.id == 0 || self.size > MAX_RANGE as u64 {
            return Err(Error::Invalid);
        }
        let mut p = Packet::new(READ_OPEN);
        p.context = context;
        p.id = self.id;
        p.arg = self.size as u32;
        p.version = self.version.value();
        p.count = 40;
        p.data[..32].copy_from_slice(&self.range_sha256);
        p.data[32..].copy_from_slice(&self.retry_epoch.value().to_le_bytes());
        Ok(p)
    }
    pub fn decode(p: &Packet) -> Result<Self, Error> {
        if p.op != READ_OPEN
            || p.status != 0
            || p.count != 40
            || p.id == 0
            || p.arg as usize > MAX_RANGE
        {
            return Err(Error::Protocol);
        }
        Ok(Self {
            id: p.id,
            size: u64::from(p.arg),
            version: Version::new(p.version).map_err(|_| Error::Protocol)?,
            range_sha256: p.data[..32].try_into().unwrap(),
            retry_epoch: Epoch::new(u64::from_le_bytes(p.data[32..].try_into().unwrap()))
                .map_err(|_| Error::Protocol)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub references: References,
    pub version: Version,
    pub size: u64,
    pub offset: u64,
    pub length: usize,
    pub range_sha256: [u8; 32],
    pub eof: bool,
    pub retry_epoch: Epoch,
}
impl Info {
    pub fn from_header(request: Request, header: Header) -> Result<Self, Error> {
        request.validate()?;
        if header.id != request.resource.object()
            || header.size > MAX_RANGE as u64
            || request.offset > header.size
            || request
                .expected_version
                .is_some_and(|version| version != header.version)
        {
            return Err(Error::Protocol);
        }
        let length = (header.size - request.offset).min(u64::from(request.length)) as usize;
        Ok(Self {
            references: References {
                workspace: request.workspace,
                resource: request.resource,
            },
            version: header.version,
            size: header.size,
            offset: request.offset,
            length,
            range_sha256: header.range_sha256,
            eof: request.offset + length as u64 == header.size,
            retry_epoch: header.retry_epoch,
        })
    }
}
