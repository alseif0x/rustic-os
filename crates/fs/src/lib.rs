// SPDX-License-Identifier: Apache-2.0
//! Bounded volume mechanics. No kernel, SDK, allocation or host filesystem dependency.
#![no_std]
#![forbid(unsafe_code)]
mod admission;
mod checksum;
mod extent;
mod format;
pub mod format7;
mod mutations;
mod namespace;
mod provision;
mod publication;
mod read;
mod recovery;
mod references;
mod storage;
mod volume;
mod volume7;
pub use admission::{Admission, AdmissionId, AdmissionState, AdmissionStatus, PreventionReason};
pub use extent::{
    DATA_SECTORS, EXTENT_DATA_BYTES, EXTENT_MAX_FILE, EXTENT_OBJECTS, EXTENTS_PER_FILE, Extent,
    Extents, FILE_SECTORS_MAX, FreeSpace, MAP_WORDS,
};
pub use namespace::{Kind, Node};
pub use publication::{Publication, PublicationCancel, PublicationPhase};
pub use recovery::{Operation, RETAINED, Receipt, Replacement, Retry};
pub use storage::{Disk, PollDisk};
pub use volume::Volume;
pub use volume7::{
    PollDisk7, PollPublication7, Publication7Cancel, Publication7Phase, Stage7, Stage7Kind,
    Volume7, WriteIdentity7,
};
pub const OBJECTS: usize = 32;
pub const MAX_FILE: usize = 1024;
pub const SECTORS: u64 = 174;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Io,
    Uncertain,
    Invalid,
    Corrupt,
    NotFound,
    Exists,
    NotDirectory,
    IsDirectory,
    NotEmpty,
    Full,
    Size,
    Version,
    ReadOnly,
    Empty,
    Exhausted,
    Unsupported,
    Lineage,
    ExpiredEpoch,
    OutcomeUnknown,
    IdempotencyConflict,
    Busy,
    Cancelled,
}
