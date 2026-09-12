// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::*;
pub(super) fn request(p: &Packet) -> Result<(), Error> {
    if p.status != 0
        || p.count as usize > DATA
        || p.data[usize::from(p.count)..].iter().any(|b| *b != 0)
    {
        return Err(Error::Protocol);
    }
    let valid = match p.op {
        LOOKUP | CREATE | MKDIR => (1..=31).contains(&p.count) && p.arg == 0 && p.version == 0,
        STAT | REMOVE | COMMIT | ABORT => p.count == 0 && p.arg == 0 && p.version == 0,
        LIST => p.count == 0 && p.version == 0,
        READ | BEGIN => p.count == 0,
        REPLACE_OPEN | admission::OPEN => p.count == 36,
        REPLACE_CHUNK | admission::CHUNK => p.count != 0 && p.version == 0,
        REPLACE_COMMIT | REPLACE_ABORT | admission::ACCEPT | admission::ABORT => {
            p.count == 0 && p.arg == 0 && p.version == 0
        }
        OPERATION_RETRY | admission::RETRY => p.count == 24 && p.arg == 0,
        OPERATION_ID
        | admission::GET
        | admission::EXECUTE
        | admission::CANCEL
        | admission::ACTIVITY
        | admission::REQUEST_CANCEL => p.count == 16 && p.id == 0 && p.arg == 0,
        admission::SCHEDULE => p.count == 16 && p.id == 0 && p.arg == 0,
        lifecycle::CANCEL => {
            if p.arg != lifecycle::VERSION {
                return Err(Error::UnsupportedVersion);
            }
            lifecycle::CancelAck::decode_request(p)?;
            true
        }
        admission::OBSERVE => {
            if !matches!(
                p.arg,
                admission::OBSERVATION_VERSION | admission::OBSERVATION_V2
            ) {
                return Err(Error::UnsupportedVersion);
            }
            p.count == 16 && p.id == 0
        }
        OPERATION_PART => p.count == 16 && p.id == 0 && matches!(p.arg, 40 | 80),
        REFERENCES => p.count == 0 && p.version == 0,
        READ_OPEN | READ_CHUNK => p.count == 30,
        RECOVERY => p.count == 0 && p.arg == 0 && p.version == 0,
        TRACK_BEGIN => p.count == 32,
        RECEIPT => p.count == 32 && p.arg == 0 && p.version == 0,
        CHUNK => p.count != 0 && p.version == 0,
        CAPABILITIES => p.count == 0 && p.id == 0 && p.arg == 0 && p.version == 0,
        negotiation::DESCRIBE => {
            negotiation::decode_request(p)?;
            true
        }
        _ => false,
    };
    if valid { Ok(()) } else { Err(Error::Protocol) }
}
