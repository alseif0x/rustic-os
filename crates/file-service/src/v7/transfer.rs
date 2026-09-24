// SPDX-License-Identifier: Apache-2.0
//! One client's streamed profile-2 replacement or admission: 40-byte packets
//! accumulate into one 512-byte sector that is handed to the volume's stage as
//! soon as it is full, so a transfer never holds more than one sector of the
//! file. The stage kind fixed at open decides which requests may continue and
//! finish it.
//!
//! The SHA-256 covers the bytes the client supplied. A fresh stage writes
//! exactly those bytes, and an exact retry only finishes when every supplied
//! byte equals the retained snapshot, so the digest is the stored content's in
//! both cases.
use crate::{disk::Synchronous, reply};
use rustic_abi::files::{Error, operation::Replacement};
use rustic_fs::{Disk, Stage7, Stage7Kind, Volume7, format7::Record7};
use sha2::{Digest, Sha256};

const SECTOR: usize = 512;

/// Why a chunk was not accepted, which decides whether the transfer survives.
pub(super) enum Fault {
    /// Refused before touching the stage; the transfer is unchanged.
    Refused(Error),
    /// The volume ended the stage; the transfer must be dropped.
    Ended(Error),
}

pub(super) struct Transfer {
    stage: Stage7,
    kind: Stage7Kind,
    request: Replacement,
    size: u32,
    received: u32,
    buffered: usize,
    block: [u8; SECTOR],
    digest: Sha256,
}

impl Transfer {
    pub(super) fn new(stage: Stage7, kind: Stage7Kind, request: Replacement, size: u32) -> Self {
        Self {
            stage,
            kind,
            request,
            size,
            received: 0,
            buffered: 0,
            block: [0; SECTOR],
            digest: Sha256::new(),
        }
    }

    pub(super) fn object(&self) -> u32 {
        self.request.resource.object()
    }

    pub(super) fn kind(&self) -> Stage7Kind {
        self.kind
    }

    pub(super) fn request(&self) -> Replacement {
        self.request
    }

    pub(super) fn complete(&self) -> bool {
        self.received == self.size
    }

    /// Accept the client bytes at `offset`, staging every sector that fills.
    pub(super) fn chunk(
        &mut self,
        volume: &mut Volume7,
        disk: &mut impl Disk,
        offset: u32,
        bytes: &[u8],
    ) -> Result<(), Fault> {
        let remaining = (self.size - self.received) as usize;
        if offset != self.received || bytes.is_empty() || bytes.len() > remaining {
            return Err(Fault::Refused(Error::Offset));
        }
        self.digest.update(bytes);
        let mut rest = bytes;
        while !rest.is_empty() {
            let staged = self.received as usize - self.buffered;
            let target = SECTOR.min(self.size as usize - staged);
            let take = (target - self.buffered).min(rest.len());
            self.block[self.buffered..self.buffered + take].copy_from_slice(&rest[..take]);
            self.buffered += take;
            self.received += take as u32;
            rest = &rest[take..];
            if self.buffered == target {
                volume
                    .stage_write(disk, &mut self.stage, &self.block[..target])
                    .map_err(|error| Fault::Ended(reply::error(error)))?;
                self.buffered = 0;
            }
        }
        Ok(())
    }

    /// Finish the complete stage and return the committed or replayed record
    /// with the SHA-256 of its bytes. Any failure leaves no stage open.
    pub(super) fn finish(
        mut self,
        volume: &mut Volume7,
        disk: &mut impl Disk,
    ) -> Result<(Record7, [u8; 32]), Error> {
        match volume.finish_tracked(disk, &mut self.stage) {
            Ok(record) => Ok((record, self.digest.finalize().into())),
            Err(error) => {
                // Only an `Invalid` refusal leaves a stage open; release it.
                let _ = volume.abort_stage(self.stage);
                Err(reply::error(error))
            }
        }
    }

    /// Finish the complete admission stage and publish it synchronously. The
    /// result is the admitted record, or for an exact retry the retained
    /// record in its current state. Any failure leaves no stage open.
    pub(super) fn finish_admission(
        mut self,
        volume: &mut Volume7,
        disk: &mut impl Disk,
    ) -> Result<Record7, Error> {
        let mut disk = Synchronous(disk);
        let refused = match volume.finish_admission(&mut disk, &mut self.stage) {
            Ok(publication) => return super::settle::settle(publication),
            Err(error) => error,
        };
        // Only an `Invalid` refusal leaves a stage open; release it.
        let _ = volume.abort_stage(self.stage);
        Err(reply::error(refused))
    }

    /// Release the stage without I/O. A stage the volume already ended is
    /// refused as stale, which leaves nothing to release.
    pub(super) fn abort(self, volume: &mut Volume7) {
        let _ = volume.abort_stage(self.stage);
    }
}
