// SPDX-License-Identifier: Apache-2.0
//! One packet, one reviewed method and one point-in-time support answer.
use crate::{
    files::{Error, Packet},
    services::{Availability, Method},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub retained_operations: u8,
    /// Total scheduling tickets, including the active ticket.
    pub execution_tickets: u8,
    pub active_publications: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub method: Method,
    pub availability: Availability,
    pub limits: Limits,
    pub contract_sha256: [u8; 32],
}

impl Descriptor {
    pub fn reviewed(
        method: Method,
        availability: Availability,
        limits: Limits,
    ) -> Result<Self, Error> {
        Ok(Self {
            method,
            availability,
            limits,
            contract_sha256: super::reviewed::digest(method)?,
        })
    }

    pub fn packet(self, context: u32) -> Result<Packet, Error> {
        self.validate()?;
        let mut p = super::request(self.method, context)?;
        p.count = 36;
        p.data[0] = self.availability as u8;
        p.data[1] = self.limits.retained_operations;
        p.data[2] = self.limits.execution_tickets;
        p.data[3] = self.limits.active_publications;
        p.data[4..36].copy_from_slice(&self.contract_sha256);
        Ok(p)
    }

    pub fn decode(p: &Packet, expected: Method) -> Result<Self, Error> {
        if p.op != super::DESCRIBE
            || p.status != 0
            || p.count != 36
            || p.id != expected as u32
            || p.version != super::VERSION
            || p.arg != super::PROFILE
            || p.data[36..] != [0; 4]
        {
            return Err(Error::Protocol);
        }
        let result = Self {
            method: super::method(p.id)?,
            availability: Availability::decode(p.data[0]).ok_or(Error::Protocol)?,
            limits: Limits {
                retained_operations: p.data[1],
                execution_tickets: p.data[2],
                active_publications: p.data[3],
            },
            contract_sha256: p.data[4..36].try_into().unwrap(),
        };
        result.validate()?;
        Ok(result)
    }

    fn validate(&self) -> Result<(), Error> {
        if self.contract_sha256 != super::reviewed::digest(self.method)?
            || !matches!(
                self.availability,
                Availability::Available | Availability::Unavailable
            )
            || self.limits.retained_operations == 0
            || self.limits.execution_tickets == 0
            || self.limits.execution_tickets > self.limits.retained_operations
            || self.limits.active_publications != 1
        {
            return Err(Error::Protocol);
        }
        Ok(())
    }
}
