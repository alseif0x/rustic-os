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
        RECOVERY => p.count == 0 && p.arg == 0 && p.version == 0,
        TRACK_BEGIN => p.count == 32,
        RECEIPT => p.count == 32 && p.arg == 0 && p.version == 0,
        CHUNK => p.count != 0 && p.version == 0,
        _ => false,
    };
    if valid { Ok(()) } else { Err(Error::Protocol) }
}
