// SPDX-License-Identifier: Apache-2.0
//! One service observation: a live execution OR an immutable retained fact.
use super::{Activity, AdmissionId, Status};
use crate::files::{Error, Packet, operation::Instance};

/// Explicit request/reply profile; this does not negotiate service-v1 support.
pub const OBSERVATION_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation {
    /// No live ticket. Admitted means prepared, not implicitly queued on restart.
    Retained(Status),
    /// Volatile progress, including settlement whose file effect is not confirmed.
    Active(Activity),
}
impl Observation {
    pub fn id(self) -> AdmissionId {
        match self {
            Self::Retained(v) => v.id,
            Self::Active(v) => v.id,
        }
    }
    pub fn service_instance(self) -> Instance {
        match self {
            Self::Retained(v) => v.service_instance,
            Self::Active(v) => v.service_instance,
        }
    }
    pub fn packet(self, context: u32) -> Result<Packet, Error> {
        let mut p = match self {
            Self::Retained(v) => v.packet(super::GET, context)?,
            Self::Active(v) => v.packet(super::ACTIVITY, context)?,
        };
        p.op = super::OBSERVE;
        p.id = OBSERVATION_VERSION;
        Ok(p)
    }
    pub fn decode(p: &Packet) -> Result<Self, Error> {
        if p.op != super::OBSERVE || p.id != OBSERVATION_VERSION {
            return Err(Error::Protocol);
        }
        let mut inner = *p;
        inner.id = 0;
        match inner.count {
            32 => {
                inner.op = super::GET;
                Status::decode(&inner).map(Self::Retained)
            }
            24 => {
                inner.op = super::ACTIVITY;
                Activity::decode(&inner).map(Self::Active)
            }
            _ => Err(Error::Protocol),
        }
    }
}
