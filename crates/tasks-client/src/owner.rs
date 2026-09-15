// SPDX-License-Identifier: Apache-2.0
//! The authorities an application lends to the owner client.
//!
//! The client performs no binding, no discovery and no rebinding of its own: it
//! borrows one already authenticated file client from the application that owns
//! it, and, when it must plan an edit, one supervisor control channel as well.
//! The two are separate traits because they are separate authorities: applying,
//! recovering and forgetting need files only, so a semantic client that may not
//! address the supervisor can still own its record and prove its outcomes.
//!
//! [`Relay`] extends [`Authority`] because planning also waits on the borrowed
//! file client's progress and keeps it usable across a supervisor job, which the
//! application performs under its own rebinding rules.
use crate::Error;
use rustic_sdk::{files::Client, rpc::Progress};

pub trait Authority {
    /// Progress behaviour of the borrowed file client.
    type Progress: Progress;
    /// The file client bound to this owner's session.
    fn files(&mut self) -> &mut Client<Self::Progress>;
}

pub trait Relay: Authority {
    /// One supervisor exchange. A refusal becomes [`Error::Service`] with the
    /// service code; accepted and started replies are returned unchanged.
    fn control(&mut self, words: [u64; 8]) -> Result<[u64; 8], Error>;
    /// Consumes a completed supervisor job reply, including any file rebinding
    /// the application performs as part of it.
    fn finish(&mut self, reply: [u64; 8]) -> Result<[u64; 8], Error>;
}
