// SPDX-License-Identifier: Apache-2.0
//! V7 does not expose the reviewed lifecycle-v1 scheduler or live cancellation.
use crate::reply;
use rustic_abi::files::{Error, Packet, negotiation};
use rustic_fs::Volume7;

/// Validate the existing DESCRIBE selector, then refuse it against a healthy
/// mounted V7 volume until the selected lifecycle contract is implemented.
pub(super) fn request(volume: &Volume7, packet: Packet) -> Result<Packet, Error> {
    negotiation::decode_request(&packet)?;
    volume.header().map_err(reply::error)?;
    Err(Error::Unavailable)
}
