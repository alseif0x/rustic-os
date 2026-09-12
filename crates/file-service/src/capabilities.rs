// SPDX-License-Identifier: Apache-2.0
//! What this service implements on the volume it has actually mounted.
use crate::Server;
use rustic_abi::{
    files::capabilities::{Bounds, Capabilities},
    services::Availability,
};
use rustic_fs::{MAX_FILE, RETAINED};

// The wire bound and the storage bound must not drift apart silently.
const _: () = assert!(MAX_FILE <= rustic_abi::files::MAX_INLINE);

impl Server {
    /// Derived from the mounted volume, never from a build-time promise. Methods
    /// owned by other services are reported unavailable here, not hidden.
    pub fn capabilities(&self) -> Capabilities {
        // Completed replacements and their receipts need the scoped format.
        let operations = if self.volume.operations_enabled().unwrap_or(false) {
            Availability::Available
        } else {
            Availability::Unavailable
        };
        Capabilities {
            availability: [
                // Only this service's own methods are listed, so the registry
                // contract is answered for a strict subset of the catalog.
                Availability::Degraded,
                // No reviewed contract digest is carried in the guest yet.
                Availability::Unavailable,
                Availability::Available,
                operations,
                operations,
                // The native live stop is a separate profile, not this method.
                Availability::Unavailable,
                Availability::Unavailable,
                Availability::Unavailable,
            ],
            bounds: Bounds {
                max_inline_bytes: MAX_FILE as u16,
                max_page_items: rustic_abi::services::METHODS as u8,
                receipt_capacity: RETAINED as u8,
            },
        }
    }
}
