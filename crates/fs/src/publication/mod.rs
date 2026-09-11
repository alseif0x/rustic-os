// SPDX-License-Identifier: Apache-2.0
//! Volatile replacement control, separate from durable service operation admission.
mod writer;
pub use writer::Publication;

/// Observable only between settled disk commands; these are not service-v1 states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationPhase {
    Preparing,
    ReadyToPublish,
    /// The header write succeeded; final flush has not yet succeeded.
    Settling,
    Committed,
    Cancelled,
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationCancel {
    /// No publication header was submitted. No durable cancellation record is stored.
    Cancelled,
    /// The header may already be visible, or this is a committed replay.
    TooLate,
}
