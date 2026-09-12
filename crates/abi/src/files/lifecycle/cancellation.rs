// SPDX-License-Identifier: Apache-2.0
//! Minimal acknowledgement under CANCEL authority; never an inspection result.
use crate::files::{Error, Packet, admission::AdmissionId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Disposition {
    Requested = 1,
    AlreadyRequested = 2,
    /// Any retained terminal result. Does not imply that a write committed.
    TooLate = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CancelAck {
    pub id: AdmissionId,
    pub disposition: Disposition,
}
impl CancelAck {
    pub fn request(id: AdmissionId, context: u32) -> Result<Packet, Error> {
        let mut p = id.packet(crate::files::admission::GET, context)?;
        p.op = super::CANCEL;
        p.arg = super::VERSION;
        Ok(p)
    }
    pub fn decode_request(p: &Packet) -> Result<AdmissionId, Error> {
        if p.op != super::CANCEL || p.arg != super::VERSION {
            return Err(Error::Protocol);
        }
        let mut inner = *p;
        inner.op = crate::files::admission::GET;
        inner.arg = 0;
        AdmissionId::decode(&inner).map_err(|_| Error::Protocol)
    }
    pub fn packet(self, context: u32) -> Result<Packet, Error> {
        let mut p = Self::request(self.id, context)?;
        p.id = super::VERSION;
        p.arg = self.disposition as u32;
        Ok(p)
    }
    pub fn decode(p: &Packet) -> Result<Self, Error> {
        if p.id != super::VERSION {
            return Err(Error::Protocol);
        }
        let disposition = match p.arg {
            1 => Disposition::Requested,
            2 => Disposition::AlreadyRequested,
            3 => Disposition::TooLate,
            _ => return Err(Error::Protocol),
        };
        let mut inner = *p;
        inner.id = 0;
        inner.arg = super::VERSION;
        Ok(Self {
            id: Self::decode_request(&inner)?,
            disposition,
        })
    }
}
