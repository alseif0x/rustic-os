// SPDX-License-Identifier: Apache-2.0
use crate::files::{
    Error, Packet,
    reference::text::{hex, lineage},
};
use core::{fmt, str::FromStr};

/// Admission identity is distinct from a completed file-operation identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionId {
    lineage: [u8; 16],
    number: u64,
}
impl AdmissionId {
    pub fn new(lineage: [u8; 16], number: u64) -> Result<Self, Error> {
        if lineage == [0; 16] || number == 0 {
            return Err(Error::Invalid);
        }
        Ok(Self { lineage, number })
    }
    pub const fn lineage(self) -> [u8; 16] {
        self.lineage
    }
    pub const fn number(self) -> u64 {
        self.number
    }
    pub fn packet(self, op: u8, context: u32) -> Result<Packet, Error> {
        if !matches!(
            op,
            super::GET | super::EXECUTE | super::CANCEL | super::SCHEDULE | super::OBSERVE
        ) && !super::live(op)
        {
            return Err(Error::Protocol);
        }
        let mut p = Packet::new(op);
        if op == super::OBSERVE {
            p.arg = super::OBSERVATION_VERSION;
        }
        p.context = context;
        p.version = self.number;
        p.count = 16;
        p.data[..16].copy_from_slice(&self.lineage);
        Ok(p)
    }
    pub fn decode(p: &Packet) -> Result<Self, Error> {
        if (!matches!(
            p.op,
            super::GET | super::EXECUTE | super::CANCEL | super::SCHEDULE | super::OBSERVE
        ) && !super::live(p.op))
            || p.status != 0
            || p.id != 0
            || if p.op == super::OBSERVE {
                !matches!(p.arg, super::OBSERVATION_VERSION | super::OBSERVATION_V2)
            } else {
                p.arg != 0
            }
            || p.count != 16
            || p.data[16..] != [0; 24]
        {
            return Err(Error::Protocol);
        }
        Self::new(p.data[..16].try_into().unwrap(), p.version)
    }
}
impl fmt::Display for AdmissionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ad_")?;
        lineage(f, self.lineage)?;
        write!(f, "_{:016x}", self.number)
    }
}
impl FromStr for AdmissionId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        let b = s.as_bytes();
        if b.len() != 52 || &b[..3] != b"ad_" || b[35] != b'_' {
            return Err(Error::Invalid);
        }
        Self::new(hex(&b[3..35])?, u64::from_be_bytes(hex(&b[36..])?))
    }
}
