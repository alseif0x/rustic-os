// SPDX-License-Identifier: Apache-2.0
//! Private decoders for the two existing completed-operation wire profiles.
//!
//! V7 stores the full-size V7 receipt internally. The profile selected by an
//! OPEN or lookup determines only the request shape and the receipt encoding
//! returned to that caller.
use rustic_abi::files::{
    DATA, Error, Packet,
    operation::{self, Lookup},
    workspace,
};

/// Existing markerless profile 1 or explicitly marked profile 2.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Profile {
    One,
    Two,
}

/// Shared, decoded OPEN fields passed from request framing into stage policy.
pub(super) struct Opening {
    pub(super) profile: Profile,
    pub(super) request: operation::Replacement,
    pub(super) size: u32,
}

/// Decode either existing replacement OPEN form into the shared identity.
///
/// Profile 1 carries the legacy 36-byte identity and its 1024-byte maximum;
/// profile 2 carries the marked 40-byte form and the larger existing limit.
pub(super) fn decode_open(p: &Packet) -> Result<Opening, Error> {
    match p.count {
        36 => Ok(Opening {
            profile: Profile::One,
            request: operation::Replacement::decode(p)?,
            size: p.arg,
        }),
        40 => {
            let decoded = workspace::Replacement::decode(p)?;
            Ok(Opening {
                profile: Profile::Two,
                request: decoded.request,
                size: p.arg,
            })
        }
        _ => Err(Error::Protocol),
    }
}

/// Decode either lookup form and normalize it to the legacy query identity.
///
/// The profile-2 marker occupies the four bytes after the profile-1 query.
/// All other request fields and reply part offsets are shared.
pub(super) fn decode_lookup(p: &Packet) -> Result<(Profile, Lookup), Error> {
    if p.status != 0 || p.count as usize > DATA {
        return Err(Error::Protocol);
    }
    let base = match p.op {
        rustic_abi::files::OPERATION_RETRY => 24,
        rustic_abi::files::OPERATION_ID | rustic_abi::files::OPERATION_PART => 16,
        _ => return Err(Error::Protocol),
    };
    let profile = match p.count as usize {
        count if count == base => Profile::One,
        count
            if count == base + 4 && p.data[base..base + 4] == workspace::PROFILE.to_le_bytes() =>
        {
            Profile::Two
        }
        _ => return Err(Error::Protocol),
    };
    if p.data[p.count as usize..].iter().any(|byte| *byte != 0)
        || (p.op == rustic_abi::files::OPERATION_PART && !matches!(p.arg, 0 | 40 | 80))
        || (p.op != rustic_abi::files::OPERATION_PART && p.arg != 0)
    {
        return Err(Error::Protocol);
    }

    let mut identity = *p;
    identity.count = base as u8;
    identity.data[base..].fill(0);
    Ok((profile, Lookup::decode(&identity)?))
}

/// Encode a canonical V7 receipt part in the requested existing profile.
///
/// A profile-1 receipt cannot describe a file over 1024 bytes. Check that
/// bound before conversion so the reply never truncates the canonical size.
pub(super) fn receipt_part(
    profile: Profile,
    receipt: workspace::Operation,
    op: u8,
    context: u32,
    offset: usize,
) -> Result<Packet, Error> {
    match profile {
        Profile::One => {
            if receipt.size > 1024 {
                return Err(Error::Size);
            }
            let legacy = operation::Operation {
                id: receipt.id,
                service_instance: receipt.service_instance,
                workspace: receipt.workspace,
                resource: receipt.resource,
                previous_version: receipt.previous_version,
                version: receipt.version,
                size: u16::try_from(receipt.size).map_err(|_| Error::Size)?,
                retry: receipt.retry,
                sha256: receipt.sha256,
            };
            legacy.part(op, context, offset)
        }
        Profile::Two => receipt.part(op, context, offset),
    }
}
