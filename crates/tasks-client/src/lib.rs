// SPDX-License-Identifier: Apache-2.0
//! Owner-side client for the native tasks application.
//!
//! The application plans an edit; this crate retains the plan, submits it once
//! and proves the outcome. It owns mechanism only: it never renders text, never
//! chooses an edit and never decides product policy. Callers supply their own
//! authority through the [`Authority`] and [`Relay`] traits, their own recovery
//! record location through [`Record`], and render the returned `Applied`,
//! `Listing`, `Recovery` and `Note` values themselves.
//!
//! The two authorities are separate on purpose. Planning needs a supervisor
//! relay; retaining, submitting, proving, recovering and forgetting need file
//! authority alone, so a client that may not address the supervisor can still
//! own a record and apply a [`Candidate`] another process planned for it.
//!
//! Planning needs no record at all, which is why [`plan`] is a free function
//! rather than a method: it retains nothing, submits nothing and reserves no
//! request identity, so a caller may plan an edit for a record it does not own
//! and hand the resulting [`Plan`] to the client that does.
//!
//! Where the planned bytes live is the caller's decision. A [`Plan`] carries its
//! own, because the planning transport assembles a document nobody lent it a
//! buffer for; a client that collects a plan in chunks lends a
//! [`CandidateBuilder`] the buffer it wants to use, which may be a block of a
//! mapped heap, and both reach the one submission path as a [`Candidate`] view
//! of those same bytes.
//!
//! The following invariants are load-bearing and are preserved verbatim from the
//! first shell implementation:
//!
//! - One unresolved intent blocks mutations. A non-empty record is unresolved
//!   until recovery or an explicit forget resolves it.
//! - The journal object may not be the target of the edit it records.
//! - `Intent::request` requires `journal_version` strictly greater than the
//!   original target version.
//! - `Intent::matches` requires an exact workspace, resource, retry tuple,
//!   previous version, size and SHA-256, plus `operation.version` greater than
//!   the numeric key and `id.sequence() == version`. An older owner operation
//!   that happens to use the same numeric key proves nothing.
//! - Cleanup uses a version-checked replacement with empty bytes and never a
//!   removal, so it cannot delete a newer owner's intent by object ID.
//! - Recovery is query-only: it looks the operation up by retry tuple and never
//!   rebuilds, resubmits or repairs the target.
//! - Only the six canonical refusals (`Full`, `Version`, `Denied`, `Revoked`,
//!   `Expired`, `ReadOnly`) prove a submission did not publish and therefore
//!   clear the journal; every other failure retains it.
//!
//! A record identified by object ID adds one rule of its own: the object must
//! already exist, this client never creates it, and, like a named record, it may
//! never be the target of the edit it records. An absent or unreadable grant is
//! a refusal, not an idle record.
#![no_std]
#![forbid(unsafe_code)]

#[cfg(all(
    target_arch = "x86_64",
    target_os = "none",
    feature = "tasks-acceptance"
))]
mod acceptance;
/// The planned candidate is pure, so it also compiles for host tests.
mod candidate;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod client;
mod error;
/// The retained record codec is pure, so it also compiles for host tests.
#[cfg(any(test, all(target_arch = "x86_64", target_os = "none")))]
mod intent;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod journal;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod mutation;
mod outcome;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod owner;
/// The record location and its rules are pure; only resolving it needs files.
mod record;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod transport;

pub use candidate::{Candidate, CandidateBuilder, Plan, Refused};
pub use error::Error;
pub use outcome::{Applied, Committed, Listing, Note, Pending, Recovery, Report};
pub use record::Record;

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use client::Client;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use mutation::Cut;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use owner::{Authority, Relay};
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use transport::plan;
