// SPDX-License-Identifier: Apache-2.0
//! Mounted V7 support for the existing reviewed lifecycle-v2 contract.
use crate::reply;
use rustic_abi::files::{Error, Packet, negotiation};
use rustic_abi::services::Availability;
use rustic_fs::Volume7;

/// Validate the existing DESCRIBE selector and report mounted execution bounds.
pub(super) fn request(volume: &Volume7, packet: Packet) -> Result<Packet, Error> {
    let method = negotiation::decode_request(&packet)?;
    volume.header().map_err(reply::error)?;
    negotiation::Descriptor::reviewed(
        method,
        Availability::Available,
        negotiation::Limits {
            retained_operations: rustic_fs::format7::RETAINED as u8,
            execution_tickets: 2,
            active_publications: 1,
        },
    )?
    .packet(packet.context)
}
