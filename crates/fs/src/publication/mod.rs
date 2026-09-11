// SPDX-License-Identifier: Apache-2.0
//! Volatile replacement control, separate from durable service operation admission.
mod command;
mod writer;
pub(crate) use command::Command;
pub use writer::Publication;

/// Local publication mechanics; these are not service-v1 operation states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationPhase {
    Preparing,
    ReadyToPublish,
    /// The header may have been submitted; final flush has not yet succeeded.
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
    /// A scratch command is outstanding. Stop after it settles successfully.
    Draining,
}
