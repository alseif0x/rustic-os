// SPDX-License-Identifier: Apache-2.0
use super::Metadata;
use crate::rpc::Rpc;
use rustic_abi::files::*;
pub struct Client<P = crate::rpc::Blocking> {
    pub(super) rpc: Rpc<P>,
    pub context: u32,
    pub(super) pending: Option<Packet>,
    pub(super) selection: super::selection::Selection,
}
impl Client {
    pub fn new(token: u64, peer: u64, context: u32) -> Self {
        Self::with_progress(token, peer, context, crate::rpc::Blocking)
    }
}
impl<P: crate::rpc::Progress> Client<P> {
    pub fn with_progress(token: u64, peer: u64, context: u32, progress: P) -> Self {
        Self {
            rpc: Rpc::with_progress(token, peer, progress),
            context,
            pending: None,
            selection: super::selection::Selection::default(),
        }
    }
    pub fn progress(&mut self) -> &mut P {
        &mut self.rpc.progress
    }
    pub fn rebind(&mut self, token: u64, peer: u64, context: u32) {
        self.rpc.rebind(token, peer);
        self.context = context;
        self.pending = None;
        self.selection = super::selection::Selection::default();
    }
    pub fn close(self) -> Result<(), crate::Error> {
        self.rpc.endpoint.close()
    }
    pub fn token(&self) -> u64 {
        self.rpc.endpoint.token()
    }
    /// Token zero represents an explicitly unbound client, not a failed exchange.
    pub(super) fn require_binding(&self) -> Result<(), Error> {
        if self.token() == 0 {
            Err(Error::Unavailable)
        } else {
            Ok(())
        }
    }
    pub fn stat(&mut self, id: u32) -> Result<Metadata, Error> {
        let mut p = Packet::new(STAT);
        p.id = id;
        Metadata::decode(self.request(p)?)
    }
    pub fn lookup(&mut self, parent: u32, name: &str) -> Result<Metadata, Error> {
        let p = self.named(LOOKUP, parent, name)?;
        Metadata::decode(self.request(p)?)
    }
    fn named(&self, op: u8, id: u32, name: &str) -> Result<Packet, Error> {
        if name.is_empty() || name.len() > 31 {
            return Err(Error::Invalid);
        }
        let mut p = Packet::new(op);
        p.id = id;
        p.count = name.len() as u8;
        p.data[..name.len()].copy_from_slice(name.as_bytes());
        Ok(p)
    }
    pub fn create(&mut self, parent: u32, name: &str, directory: bool) -> Result<Metadata, Error> {
        let p = self.named(if directory { MKDIR } else { CREATE }, parent, name)?;
        Metadata::decode(self.request(p)?)
    }
    pub fn list(&mut self, id: u32, cursor: u8) -> Result<Option<Metadata>, Error> {
        let mut p = Packet::new(LIST);
        p.id = id;
        p.arg = cursor as u32;
        let r = self.request(p)?;
        if r.id == 0 {
            Ok(None)
        } else {
            Metadata::decode(r).map(Some)
        }
    }
    pub fn remove(&mut self, id: u32) -> Result<(), Error> {
        let mut p = Packet::new(REMOVE);
        p.id = id;
        self.request(p)?;
        Ok(())
    }
    pub fn read(&mut self, id: u32, bytes: &mut [u8]) -> Result<usize, Error> {
        let m = self.stat(id)?;
        if m.directory {
            return Err(Error::IsDirectory);
        }
        if m.length > bytes.len() {
            return Err(Error::Size);
        }
        let mut offset = 0;
        while offset < m.length {
            let mut p = Packet::new(READ);
            p.id = id;
            p.arg = offset as u32;
            p.version = m.version;
            let r = self.request(p)?;
            let n = r.count as usize;
            if r.id != id
                || r.version != m.version
                || r.arg as usize != m.length
                || n == 0
                || offset + n > m.length
            {
                return Err(Error::Protocol);
            }
            bytes[offset..offset + n].copy_from_slice(r.payload());
            offset += n;
        }
        Ok(offset)
    }
    pub fn replace(&mut self, id: u32, version: u64, bytes: &[u8]) -> Result<Metadata, Error> {
        if bytes.len() > 1024 {
            return Err(Error::Size);
        }
        let mut p = Packet::new(BEGIN);
        p.id = id;
        p.arg = bytes.len() as u32;
        p.version = version;
        self.request(p)?;
        let result = (|| {
            for (index, chunk) in bytes.chunks(DATA).enumerate() {
                let mut p = Packet::new(CHUNK);
                p.id = id;
                p.arg = (index * DATA) as u32;
                p.count = chunk.len() as u8;
                p.data[..chunk.len()].copy_from_slice(chunk);
                self.request(p)?;
            }
            let mut p = Packet::new(COMMIT);
            p.id = id;
            Metadata::decode(self.request(p)?)
        })();
        if result
            .as_ref()
            .is_err_and(|e| !matches!(e, Error::Uncertain | Error::Protocol | Error::Closed))
        {
            let mut p = Packet::new(ABORT);
            p.id = id;
            let _ = self.request(p);
        }
        result
    }
}
