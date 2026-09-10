// SPDX-License-Identifier: Apache-2.0
//! Pure owned progress over a caller buffer; transport owns peer and correlation checks.
use rustic_abi::files::{
    DATA, Error, Packet, READ_CHUNK,
    read::{Header, Info, MAX_RANGE, Request},
};
use sha2::{Digest, Sha256};

pub(super) struct Collector<'a> {
    request: Request,
    context: u32,
    output: &'a mut [u8],
    info: Option<Info>,
    used: usize,
    hash: Sha256,
    failed: bool,
    complete: bool,
}
impl<'a> Collector<'a> {
    pub(super) fn new(request: Request, context: u32, output: &'a mut [u8]) -> Result<Self, Error> {
        output.fill(0);
        request.validate()?;
        if output.len() < usize::from(request.length) || output.len() > MAX_RANGE {
            return Err(Error::Size);
        }
        Ok(Self {
            request,
            context,
            output,
            info: None,
            used: 0,
            hash: Sha256::new(),
            failed: false,
            complete: false,
        })
    }
    fn fail<T>(&mut self) -> Result<T, Error> {
        self.failed = true;
        self.output.fill(0);
        Err(Error::Protocol)
    }
    pub(super) fn open(&mut self, reply: Packet) -> Result<(), Error> {
        if self.failed || self.info.is_some() || reply.context != self.context {
            return self.fail();
        }
        let info =
            Header::decode(&reply).and_then(|header| Info::from_header(self.request, header));
        match info {
            Ok(info) => {
                self.info = Some(info);
                Ok(())
            }
            Err(_) => self.fail(),
        }
    }
    pub(super) fn next(&self) -> Result<Option<Packet>, Error> {
        if self.failed {
            return Err(Error::Protocol);
        }
        let info = self.info.ok_or(Error::Protocol)?;
        if self.used == info.length {
            return Ok(None);
        }
        let request = Request {
            expected_version: Some(info.version),
            offset: self.request.offset + self.used as u64,
            length: (info.length - self.used).min(DATA) as u16,
            ..self.request
        };
        request.packet(READ_CHUNK, self.context).map(Some)
    }
    pub(super) fn chunk(&mut self, reply: Packet) -> Result<(), Error> {
        let Ok(Some(request)) = self.next() else {
            return self.fail();
        };
        let info = self.info.unwrap();
        let length = reply.count as usize;
        if reply.op != READ_CHUNK
            || reply.status != 0
            || reply.context != self.context
            || reply.id != request.id
            || reply.version != info.version.value()
            || u64::from(reply.arg) != info.size
            || length != request.arg as usize
            || reply.data[length..].iter().any(|b| *b != 0)
        {
            return self.fail();
        }
        let bytes = &reply.data[..length];
        self.output[self.used..self.used + length].copy_from_slice(bytes);
        self.hash.update(bytes);
        self.used += length;
        Ok(())
    }
    pub(super) fn finish(mut self) -> Result<Info, Error> {
        let Some(info) = self.info else {
            return self.fail();
        };
        let actual: [u8; 32] = self.hash.clone().finalize().into();
        if self.failed || self.used != info.length || actual != info.range_sha256 {
            return self.fail();
        }
        self.complete = true;
        Ok(info)
    }
}
impl Drop for Collector<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.output.fill(0);
        }
    }
}
