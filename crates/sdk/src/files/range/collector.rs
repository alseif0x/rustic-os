// SPDX-License-Identifier: Apache-2.0
//! Pure owned progress over one bounded, version-pinned range.
use rustic_abi::files::{
    DATA, Error, Packet, READ_CHUNK, READ_OPEN,
    read::{Header, Info, MAX_RANGE, Request},
};
use sha2::{Digest, Sha256};

pub(super) struct Collector {
    request: Request,
    context: u32,
    bytes: [u8; MAX_RANGE],
    info: Option<Info>,
    used: usize,
    hash: Sha256,
    failed: bool,
    complete: bool,
    #[cfg(test)]
    drop_probe: Option<&'static core::sync::atomic::AtomicBool>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct VerifiedRange {
    info: Info,
    bytes: [u8; MAX_RANGE],
}

pub(super) fn read_into<F>(
    request: Request,
    context: u32,
    out: &mut [u8],
    mut exchange: F,
) -> Result<Info, Error>
where
    F: FnMut(Packet) -> Result<Packet, Error>,
{
    out.fill(0);
    request.validate()?;
    if out.len() < usize::from(request.length) || out.len() > MAX_RANGE {
        return Err(Error::Size);
    }
    let result = (|| {
        let mut collector = Collector::new(request, context)?;
        collector.open(exchange(request.packet(READ_OPEN, context)?)?)?;
        while let Some(chunk) = collector.next()? {
            collector.chunk(exchange(chunk)?)?;
        }
        collector.finish()
    })();
    match result {
        Ok(verified) => {
            let info = verified.info();
            out[..info.length].copy_from_slice(verified.bytes());
            Ok(info)
        }
        Err(error) => {
            out.fill(0);
            Err(error)
        }
    }
}

impl Collector {
    pub(super) fn new(request: Request, context: u32) -> Result<Self, Error> {
        request.validate()?;
        Ok(Self {
            request,
            context,
            bytes: [0; MAX_RANGE],
            info: None,
            used: 0,
            hash: Sha256::new(),
            failed: false,
            complete: false,
            #[cfg(test)]
            drop_probe: None,
        })
    }

    #[cfg(test)]
    pub(super) fn observe_drop(&mut self, probe: &'static core::sync::atomic::AtomicBool) {
        self.drop_probe = Some(probe);
    }

    #[cfg(test)]
    pub(super) fn buffered(&self) -> &[u8; MAX_RANGE] {
        &self.bytes
    }

    fn fail<T>(&mut self) -> Result<T, Error> {
        self.failed = true;
        self.bytes.fill(0);
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
            || length > DATA
            || reply.data[length..].iter().any(|byte| *byte != 0)
        {
            return self.fail();
        }
        let bytes = &reply.data[..length];
        self.bytes[self.used..self.used + length].copy_from_slice(bytes);
        self.hash.update(bytes);
        self.used += length;
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<VerifiedRange, Error> {
        let Some(info) = self.info else {
            return self.fail();
        };
        let actual: [u8; 32] = self.hash.clone().finalize().into();
        if self.failed || self.used != info.length || actual != info.range_sha256 {
            return self.fail();
        }
        self.complete = true;
        Ok(VerifiedRange {
            info,
            bytes: core::mem::replace(&mut self.bytes, [0; MAX_RANGE]),
        })
    }
}

impl VerifiedRange {
    pub fn info(&self) -> Info {
        self.info
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.info.length]
    }
}

impl Drop for Collector {
    fn drop(&mut self) {
        if !self.complete {
            self.bytes.fill(0);
        }
        #[cfg(test)]
        if let Some(probe) = self.drop_probe {
            probe.store(
                self.bytes.iter().all(|byte| *byte == 0),
                core::sync::atomic::Ordering::Relaxed,
            );
        }
    }
}

impl Drop for VerifiedRange {
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}
