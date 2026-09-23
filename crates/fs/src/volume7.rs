// SPDX-License-Identifier: Apache-2.0
//! In-place owner for provisioning, mounting and tracked v7 file replacement.
//!
//! Mount verifies both header copies, raw aggregate checksums, the decoded
//! generation, and every live or retained payload CRC. Mutation paths currently
//! support existing-file direct commits with retained exact payload snapshots
//! and explicit terminal-record retention maintenance. Staged admission and
//! cancellation, migration, service integration and capability advertisement
//! remain separate work.

use crate::extent::MAP_WORDS;
use crate::format7::{Header7, MAP_WORDS as FORMAT_MAP_WORDS, NODES, Node7, RETAINED, Record7};
use crate::{Error, Kind};

mod maintenance;
mod mount;
mod payload;
mod provision;
mod publication;
mod replacement;

/// Scoped retry identity for one direct v7 file replacement.
///
/// The service owns the meaning and authority of these identifiers. The volume
/// persists them and enforces their nonzero and version-domain constraints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteIdentity7 {
    pub subject: u64,
    pub workspace: u32,
    pub object: u32,
    pub instance: u64,
    pub retry_epoch: u64,
    pub retry_key: u64,
}

/// A v7 volume owner. Its decoded state and validator workspace are private;
/// callers use accessors and mutation methods only after a successful provision
/// or mount. Keep this fixed, multi-array storage in a long-lived owner rather
/// than a constrained kernel stack frame; `EMPTY` can initialize static storage.
pub struct Volume7 {
    pub(super) header: Header7,
    pub(super) nodes: [Node7; NODES],
    pub(super) records: [Option<Record7>; RETAINED],
    pub(super) map: [u64; MAP_WORDS],
    pub(super) validation: [u64; MAP_WORDS],
    pub(super) recovered_from_header: bool,
    pub(super) fenced: bool,
}

impl Volume7 {
    /// Empty fenced storage for in-place provisioning or mounting.
    pub const EMPTY: Self = Self {
        header: Header7::initial([0; 16]),
        nodes: [Node7::EMPTY; NODES],
        records: [None; RETAINED],
        map: [0; MAP_WORDS],
        validation: [0; MAP_WORDS],
        recovered_from_header: false,
        fenced: true,
    };

    fn ready(&self) -> Result<(), Error> {
        if self.fenced {
            Err(Error::Uncertain)
        } else {
            Ok(())
        }
    }

    /// Clear partial or stale state while keeping this value fenced.
    pub(super) fn clear(&mut self) {
        self.header = Header7::initial([0; 16]);
        self.nodes.fill(Node7::EMPTY);
        self.records.fill(None);
        self.map.fill(0);
        self.validation.fill(0);
        self.recovered_from_header = false;
    }

    /// The verified selected header.
    pub fn header(&self) -> Result<&Header7, Error> {
        self.ready()?;
        Ok(&self.header)
    }

    /// All verified node slots, including canonical empty slots.
    pub fn nodes(&self) -> Result<&[Node7; NODES], Error> {
        self.ready()?;
        Ok(&self.nodes)
    }

    /// Find one live node by persistent identity.
    pub fn node(&self, id: u32) -> Result<Option<&Node7>, Error> {
        self.ready()?;
        Ok(self
            .nodes
            .iter()
            .find(|node| node.kind != Kind::Empty && node.id == id))
    }

    /// Verified retained records, including empty slots.
    pub fn retained_records(&self) -> Result<&[Option<Record7>; RETAINED], Error> {
        self.ready()?;
        Ok(&self.records)
    }

    /// Verified allocation bitmap, where a set bit means allocated.
    pub fn allocation_map(&self) -> Result<&[u64; FORMAT_MAP_WORDS], Error> {
        self.ready()?;
        Ok(&self.map)
    }

    /// Number of payload sectors the verified map leaves free.
    pub fn free_sectors(&self) -> Result<u64, Error> {
        self.ready()?;
        Ok(self
            .map
            .iter()
            .map(|word| u64::from(word.count_zeros()))
            .sum())
    }

    /// Whether a nonzero invalid header copy was ignored during the last
    /// successful mount. Provisioning starts with both copies zeroed.
    pub fn recovered_from_header(&self) -> Result<bool, Error> {
        self.ready()?;
        Ok(self.recovered_from_header)
    }
}
