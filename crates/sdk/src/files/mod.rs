// SPDX-License-Identifier: Apache-2.0
//! Native file client. No path authority is inferred from strings or manifests.
mod client;
mod metadata;
mod paths;
pub use client::Client;
pub use metadata::Metadata;
pub use rustic_abi::files::{Error, Packet};
