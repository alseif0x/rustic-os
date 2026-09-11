// SPDX-License-Identifier: Apache-2.0
//! One owned command encoded by the shared publication ordering.
use crate::{Disk, Error, PollDisk};
use core::task::Poll;

// One stack-owned sector is intentional: the no_std filesystem has no allocator.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Command {
    Write(u64, [u8; 512]),
    Flush,
}
impl Command {
    pub(crate) fn execute(&self, disk: &mut impl Disk) -> Result<(), Error> {
        match self {
            Self::Write(sector, bytes) => disk.write(*sector, bytes),
            Self::Flush => disk.flush(),
        }
    }
    pub(super) fn poll(&self, disk: &mut impl PollDisk) -> Poll<Result<(), Error>> {
        match self {
            Self::Write(sector, bytes) => disk.poll_write(*sector, bytes),
            Self::Flush => disk.poll_flush(),
        }
    }
}
