// SPDX-License-Identifier: Apache-2.0
//! Bounded volume mechanics. No kernel, SDK, allocation or host filesystem dependency.
#![no_std]
#![forbid(unsafe_code)]
mod admission;
mod checksum;
mod extent;
mod format;
mod format6;
mod mutations;
mod namespace;
mod provision;
mod publication;
mod read;
mod receipt6;
mod recovery;
mod references;
mod storage;
mod upgrade6;
mod volume;
mod volume6;
pub use admission::{Admission, AdmissionId, AdmissionState, AdmissionStatus, PreventionReason};
pub use extent::{
    DATA_BYTES_V6, DATA_SECTORS, EXTENTS_PER_FILE, Extent, Extents, FILE_SECTORS_MAX, FreeSpace,
    MAP_WORDS, MAX_FILE_V6, OBJECTS_V6,
};
pub use format6::{
    GENERATIONS, Header6, MAGIC, MAP_SECTORS, NODE_BYTES, NODES_SECTORS, Node6, PAYLOAD_SECTOR,
    RECEIPTS_SECTORS, VOLUME_SECTORS, map_sector, nodes_sector, receipts_sector,
};
pub use namespace::{Kind, Node};
pub use publication::{Publication, PublicationCancel, PublicationPhase};
pub use receipt6::{RECEIPT_BYTES, RECEIPT_SECTORS, RETAINED_V6, Receipt6, Receipts6};
pub use recovery::{Operation, RETAINED, Receipt, Replacement, Retry};
pub use storage::{Disk, PollDisk};
pub use upgrade6::{Report as UpgradeReport, UpgradedVolume, upgrade as upgrade6};
pub use volume::Volume;
pub use volume6::{Volume6, mount as mount6, provision as provision6};
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
