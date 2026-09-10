// SPDX-License-Identifier: Apache-2.0
//! Bounded volume mechanics. No kernel, SDK, allocation or host filesystem dependency.
#![no_std]
#![forbid(unsafe_code)]
mod checksum;
mod format;
mod namespace;
mod storage;
mod volume;
pub use namespace::{Kind, Node};
pub use storage::Disk;
pub use volume::Volume;
pub const OBJECTS: usize = 32;
pub const MAX_FILE: usize = 1024;
pub const SECTORS: u64 = 160;
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
}
