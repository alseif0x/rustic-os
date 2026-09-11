// SPDX-License-Identifier: Apache-2.0
//! Durable storage facts, not authorization or a public asynchronous service API.
mod codec;
mod query;
mod transition;

use crate::{Receipt, Replacement};

/// Separate namespace from completed-operation IDs. The number is the sequence
/// of a durably published admission, never an uncommitted future file version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionId {
    pub lineage: [u8; 16],
    pub number: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionState {
    Admitted,
    Cancelled,
    Committed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionStatus {
    pub id: AdmissionId,
    pub state: AdmissionState,
    /// Sequence of the terminal metadata transition; zero while admitted.
    pub terminal: u64,
}

#[derive(Debug)]
pub struct Admission<'a> {
    pub status: AdmissionStatus,
    pub request: Replacement,
    pub instance: u64,
    pub receipt: Option<Receipt>,
    pub bytes: &'a [u8],
}

#[derive(Clone, Copy)]
pub(crate) struct Stored {
    pub(crate) number: u64,
    pub(crate) state: AdmissionState,
    pub(crate) terminal: u64,
}
