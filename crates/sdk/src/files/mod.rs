// SPDX-License-Identifier: Apache-2.0
//! Native file client. No path authority is inferred from strings or manifests.
mod admissions;
mod client;
mod discovery;
mod lifecycle_client;
mod negotiation;
pub use negotiation::LifecycleBinding;
mod operations;
mod range;
mod recovery;
mod references;
mod staging;
mod transport;
pub use rustic_abi::files::recovery::{Receipt, Retry};
mod metadata;
mod paths;
pub use client::Client;
pub use metadata::Metadata;
pub use rustic_abi::files::{Error, Packet};
pub use rustic_abi::files::{admission, capabilities, lifecycle, operation, read, reference};
