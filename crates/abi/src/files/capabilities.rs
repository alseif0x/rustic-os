// SPDX-License-Identifier: Apache-2.0
//! One bounded report of what this service implements. It carries no file content,
//! identity or authority: availability is not permission and never grants a right.
use crate::files::{Error, Packet};
use crate::services::{Availability, METHODS, Method};

/// Limits the answering service actually enforces, reported as facts about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub max_inline_bytes: u16,
    pub max_page_items: u8,
    pub receipt_capacity: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// One entry per catalog method, in catalog order.
    pub availability: [Availability; METHODS],
    pub bounds: Bounds,
}

impl Capabilities {
    pub fn of(self, method: Method) -> Availability {
        self.availability[method as usize - 1]
    }

    pub fn packet(self, context: u32) -> Result<Packet, Error> {
        if self.bounds.max_inline_bytes as usize > super::MAX_INLINE
            || self.bounds.max_page_items == 0
            || self.bounds.receipt_capacity == 0
        {
            return Err(Error::Protocol);
        }
        let mut p = Packet::new(super::CAPABILITIES);
        p.context = context;
        p.version = u64::from(crate::services::VERSION);
        p.arg = u32::from(self.bounds.max_inline_bytes)
            | (u32::from(self.bounds.max_page_items) << 16)
            | (u32::from(self.bounds.receipt_capacity) << 24);
        p.count = METHODS as u8;
        for (slot, availability) in p.data[..METHODS].iter_mut().zip(self.availability) {
            *slot = availability as u8;
        }
        Ok(p)
    }

    pub fn decode(p: &Packet) -> Result<Self, Error> {
        if p.op != super::CAPABILITIES
            || p.status != 0
            || p.id != 0
            || p.count != METHODS as u8
            || p.version != u64::from(crate::services::VERSION)
            || p.data[METHODS..] != [0; super::DATA - METHODS]
        {
            return Err(Error::Protocol);
        }
        let mut availability = [Availability::Unavailable; METHODS];
        for (slot, byte) in availability.iter_mut().zip(&p.data[..METHODS]) {
            *slot = Availability::decode(*byte).ok_or(Error::Protocol)?;
        }
        let bounds = Bounds {
            max_inline_bytes: (p.arg & 0xffff) as u16,
            max_page_items: ((p.arg >> 16) & 0xff) as u8,
            receipt_capacity: (p.arg >> 24) as u8,
        };
        if bounds.max_inline_bytes as usize > super::MAX_INLINE
            || bounds.max_page_items == 0
            || bounds.receipt_capacity == 0
        {
            return Err(Error::Protocol);
        }
        Ok(Self {
            availability,
            bounds,
        })
    }
}
