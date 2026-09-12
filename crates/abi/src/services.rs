// SPDX-License-Identifier: Apache-2.0
//! Shared identity of the logical service-v1 methods. Identity is not implementation:
//! a method listed here may be unavailable in every running service.
pub const VERSION: u8 = 1;

/// Catalog order is part of the shared contract; ids are never reused or reordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Method {
    CapabilitiesList = 1,
    CapabilitiesDescribe = 2,
    FilesRead = 3,
    FilesReplace = 4,
    OperationsGet = 5,
    OperationsCancel = 6,
    EventsRead = 7,
    SystemStatus = 8,
}

pub const METHODS: usize = 8;

impl Method {
    pub const ALL: [Self; METHODS] = [
        Self::CapabilitiesList,
        Self::CapabilitiesDescribe,
        Self::FilesRead,
        Self::FilesReplace,
        Self::OperationsGet,
        Self::OperationsCancel,
        Self::EventsRead,
        Self::SystemStatus,
    ];

    pub fn decode(value: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|m| *m as u8 == value)
    }

    /// The canonical name, so a client never has to infer it from an integer.
    pub fn name(self) -> &'static str {
        match self {
            Self::CapabilitiesList => "capabilities.list",
            Self::CapabilitiesDescribe => "capabilities.describe",
            Self::FilesRead => "files.read",
            Self::FilesReplace => "files.replace",
            Self::OperationsGet => "operations.get",
            Self::OperationsCancel => "operations.cancel",
            Self::EventsRead => "events.read",
            Self::SystemStatus => "system.status",
        }
    }
}

/// What the answering service implements, never what the caller may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Availability {
    /// Implemented within its documented profile.
    Available = 1,
    /// Implemented for a strict subset of the contract; the profile states which.
    Degraded = 2,
    /// Not implemented here. Another service may still own it.
    Unavailable = 3,
}

impl Availability {
    pub fn decode(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Available),
            2 => Some(Self::Degraded),
            3 => Some(Self::Unavailable),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Degraded => "degraded",
            Self::Unavailable => "unavailable",
        }
    }
}
