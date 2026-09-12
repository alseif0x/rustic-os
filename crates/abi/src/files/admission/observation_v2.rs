// SPDX-License-Identifier: Apache-2.0
//! Explicit cause-aware observation. Profile 1 remains a separate strict codec.
use super::{Activity, AdmissionId, Observation, State, Status};
use crate::files::{Error, Packet};

pub const OBSERVATION_V2: u32 = 2;

/// Public wire codes, independent of the filesystem's private encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PreventionReason {
    Unknown = 1,
    Requested = 2,
    VersionConflict = 3,
    AuthorityLost = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationV2 {
    /// A cause exists exactly when storage confirms Cancelled. Unknown is a
    /// retained legacy fact, not permission to infer a requested cancellation.
    Retained {
        status: Status,
        prevention: Option<PreventionReason>,
    },
    /// A live stop latch never establishes a retained prevention cause.
    Active(Activity),
}
impl ObservationV2 {
    pub fn request(id: AdmissionId, context: u32) -> Result<Packet, Error> {
        let mut p = id.packet(super::OBSERVE, context)?;
        p.arg = OBSERVATION_V2;
        Ok(p)
    }
    /// Explicit projection for a peer that requested the older profile.
    pub fn coarse(self) -> Observation {
        match self {
            Self::Retained { status, .. } => Observation::Retained(status),
            Self::Active(v) => Observation::Active(v),
        }
    }
    pub fn packet(self, context: u32) -> Result<Packet, Error> {
        let mut p = self.coarse().packet(context)?;
        p.id = OBSERVATION_V2;
        if let Self::Retained { status, prevention } = self {
            if (status.state == State::Cancelled) != prevention.is_some() {
                return Err(Error::Protocol);
            }
            p.count = 33;
            p.data[32] = prevention.map_or(0, |v| v as u8);
        }
        Ok(p)
    }
    pub fn decode(p: &Packet) -> Result<Self, Error> {
        if p.op != super::OBSERVE || p.id != OBSERVATION_V2 {
            return Err(Error::Protocol);
        }
        let mut inner = *p;
        inner.id = super::OBSERVATION_VERSION;
        match p.count {
            24 => match Observation::decode(&inner)? {
                Observation::Active(v) => Ok(Self::Active(v)),
                _ => Err(Error::Protocol),
            },
            33 => {
                let prevention = match p.data[32] {
                    0 => None,
                    1 => Some(PreventionReason::Unknown),
                    2 => Some(PreventionReason::Requested),
                    3 => Some(PreventionReason::VersionConflict),
                    4 => Some(PreventionReason::AuthorityLost),
                    _ => return Err(Error::Protocol),
                };
                inner.count = 32;
                inner.data[32] = 0;
                let Observation::Retained(status) = Observation::decode(&inner)? else {
                    return Err(Error::Protocol);
                };
                if (status.state == State::Cancelled) != prevention.is_some() {
                    return Err(Error::Protocol);
                }
                Ok(Self::Retained { status, prevention })
            }
            _ => Err(Error::Protocol),
        }
    }
}
