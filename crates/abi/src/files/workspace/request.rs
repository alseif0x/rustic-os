// SPDX-License-Identifier: Apache-2.0
use super::{MAX_FILE_BYTES, PROFILE};
use crate::files::{Error, OPERATION_PART, Packet, operation};

/// The shared resource/retry identity, selected explicitly for large payloads.
#[derive(Clone, Copy, Debug)]
pub struct Replacement {
    pub request: operation::Replacement,
}
impl Replacement {
    pub fn packet(self, size: usize, context: u32) -> Result<Packet, Error> {
        if size > MAX_FILE_BYTES as usize {
            return Err(Error::Size);
        }
        // Reuse identity encoding, not the legacy payload-size policy.
        let mut p = self.request.packet(0, context)?;
        p.arg = size as u32;
        p.count = 40;
        p.data[36..40].copy_from_slice(&PROFILE.to_le_bytes());
        Ok(p)
    }
    pub fn decode(p: &Packet) -> Result<Self, Error> {
        if p.count != 40 || p.arg > MAX_FILE_BYTES || p.data[36..40] != PROFILE.to_le_bytes() {
            return Err(Error::Protocol);
        }
        let mut identity = *p;
        identity.count = 36;
        identity.arg = 0;
        identity.data[36..40].fill(0);
        Ok(Self {
            request: operation::Replacement::decode(&identity)?,
        })
    }
}

/// Read-only historical lookup with an explicit profile, including later parts.
#[derive(Clone, Copy, Debug)]
pub struct Lookup {
    pub query: operation::Lookup,
}
impl Lookup {
    pub fn packet(self, context: u32) -> Packet {
        let mut p = self.query.packet(context);
        let end = p.count as usize;
        p.data[end..end + 4].copy_from_slice(&PROFILE.to_le_bytes());
        p.count += 4;
        p
    }
    pub fn decode(p: &Packet) -> Result<Self, Error> {
        let end = match p.op {
            crate::files::OPERATION_RETRY => 24,
            crate::files::OPERATION_ID | OPERATION_PART => 16,
            _ => return Err(Error::Protocol),
        };
        if p.status != 0
            || p.count as usize != end + 4
            || p.data[end..end + 4] != PROFILE.to_le_bytes()
            || p.data[end + 4..].iter().any(|b| *b != 0)
            || (p.op == OPERATION_PART && !matches!(p.arg, 0 | 40 | 80))
            || (p.op != OPERATION_PART && p.arg != 0)
        {
            return Err(Error::Protocol);
        }
        let mut identity = *p;
        identity.count = end as u8;
        identity.data[end..].fill(0);
        Ok(Self {
            query: operation::Lookup::decode(&identity)?,
        })
    }
}
