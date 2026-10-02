// SPDX-License-Identifier: Apache-2.0
//! Mounted V7 implementation discovery.
use crate::reply;
use rustic_abi::{
    files::{
        Error, MAX_INLINE, Packet,
        capabilities::{Bounds, Capabilities},
    },
    services::{Availability, METHODS},
};
use rustic_fs::{Volume7, format7::RETAINED};

/// Report the implementation mounted by this service instance. Discovery is
/// independent of a client's rights, but only a healthy mounted header can
/// make an implementation claim.
pub(super) fn request(volume: &Volume7, packet: Packet) -> Result<Packet, Error> {
    volume.header().map_err(reply::error)?;

    let capabilities = Capabilities {
        availability: [
            // The catalogue is answered for this service's strict subset.
            Availability::Degraded,
            // V7 has no reviewed lifecycle descriptor implementation.
            Availability::Unavailable,
            Availability::Available,
            Availability::Available,
            Availability::Available,
            Availability::Unavailable,
            Availability::Unavailable,
            Availability::Unavailable,
        ],
        bounds: Bounds {
            max_inline_bytes: MAX_INLINE as u16,
            max_page_items: METHODS as u8,
            receipt_capacity: RETAINED as u8,
        },
    };
    capabilities
        .packet(packet.context)
        .map_err(|_| Error::Protocol)
}
