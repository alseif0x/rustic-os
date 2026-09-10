// SPDX-License-Identifier: Apache-2.0
use super::{Client, Receipt, Retry};
use rustic_abi::files::*;
impl<P: crate::rpc::Progress> Client<P> {
    /// Caller chooses a nonzero key and retains this complete token before submitting.
    pub fn retry_token(&mut self, id: u32, key: u64) -> Result<Retry, Error> {
        if key == 0 {
            return Err(Error::Invalid);
        }
        let mut p = Packet::new(RECOVERY);
        p.id = id;
        let r = self.request(p)?;
        if r.count != 24 || r.arg != 2 || r.id != 0 || r.version != 0 {
            return Err(Error::Protocol);
        }
        let mut bytes = [0; 32];
        bytes[..24].copy_from_slice(r.payload());
        bytes[24..].copy_from_slice(&key.to_le_bytes());
        Retry::decode(&bytes)
    }
    pub fn receipt(&mut self, id: u32, retry: Retry) -> Result<Receipt, Error> {
        let mut p = Packet::new(RECEIPT);
        p.id = id;
        p.count = 32;
        p.data[..32].copy_from_slice(&retry.encode());
        let receipt = Receipt::decode(self.request(p)?)?;
        if receipt.id != id || receipt.retry != retry {
            return Err(Error::Protocol);
        }
        Ok(receipt)
    }
    /// Whole-file replacement. No implicit retry, key generation, receipt eviction or new version lookup.
    pub fn replace_tracked(
        &mut self,
        id: u32,
        version: u64,
        retry: Retry,
        bytes: &[u8],
    ) -> Result<Receipt, Error> {
        self.stage_tracked(id, version, retry, bytes)?;
        let result = (|| {
            let mut p = Packet::new(COMMIT);
            p.id = id;
            let receipt = Receipt::decode(self.request(p)?).map_err(|_| Error::Uncertain)?;
            if receipt.id != id
                || receipt.previous != version
                || receipt.retry != retry
                || receipt.length as usize != bytes.len()
            {
                return Err(Error::Uncertain);
            }
            Ok(receipt)
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
    /// Stages bounded bytes only; no durable acceptance or file effect until COMMIT.
    pub fn stage_tracked(
        &mut self,
        id: u32,
        version: u64,
        retry: Retry,
        bytes: &[u8],
    ) -> Result<(), Error> {
        if bytes.len() > 1024 {
            return Err(Error::Size);
        }
        let mut p = Packet::new(TRACK_BEGIN);
        p.id = id;
        p.arg = bytes.len() as u32;
        p.version = version;
        p.count = 32;
        p.data[..32].copy_from_slice(&retry.encode());
        self.request(p)?;
        for (index, chunk) in bytes.chunks(DATA).enumerate() {
            let mut p = Packet::new(CHUNK);
            p.id = id;
            p.arg = (index * DATA) as u32;
            p.count = chunk.len() as u8;
            p.data[..chunk.len()].copy_from_slice(chunk);
            self.request(p)?;
        }
        Ok(())
    }
}
