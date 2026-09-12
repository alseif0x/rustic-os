// SPDX-License-Identifier: Apache-2.0
//! Projection of retained service facts into the explicitly requested profile.
use rustic_abi::files::{Error, Packet, admission as a};

pub(super) fn reason(value: rustic_fs::PreventionReason) -> a::PreventionReason {
    use rustic_fs::PreventionReason as R;
    match value {
        R::Unknown => a::PreventionReason::Unknown,
        R::Requested => a::PreventionReason::Requested,
        R::VersionConflict => a::PreventionReason::VersionConflict,
        R::AuthorityLost => a::PreventionReason::AuthorityLost,
    }
}

pub(super) fn reply(view: a::ObservationV2, request: Packet) -> Result<Packet, Error> {
    match request.arg {
        a::OBSERVATION_VERSION => view.coarse().packet(request.context),
        a::OBSERVATION_V2 => view.packet(request.context),
        _ => Err(Error::UnsupportedVersion),
    }
}
