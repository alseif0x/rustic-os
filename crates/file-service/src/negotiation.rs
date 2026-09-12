// SPDX-License-Identifier: Apache-2.0
//! Read-only mounted support; no persistent instance allocation or authority grant.
use crate::Server;
use rustic_abi::{
    files::{
        Error, Packet,
        negotiation::{self, Descriptor, Limits},
    },
    services::Availability,
};

impl Server {
    pub(super) fn lifecycle_descriptor(&self, request: &Packet) -> Result<Packet, Error> {
        let method = negotiation::decode_request(request)?;
        let availability = match self.volume.retained_admission(0) {
            Ok(_) => Availability::Available,
            Err(rustic_fs::Error::Unsupported) => Availability::Unavailable,
            // A fenced/uncertain volume must not advertise a healthy binding.
            Err(error) => return Err(crate::reply::error(error)),
        };
        Descriptor::reviewed(
            method,
            availability,
            Limits {
                retained_operations: rustic_fs::RETAINED as u8,
                execution_tickets: rustic_fs::RETAINED as u8,
                active_publications: 1,
            },
        )?
        .packet(request.context)
    }
}
