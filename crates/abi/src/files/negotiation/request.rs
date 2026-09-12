// SPDX-License-Identifier: Apache-2.0
//! Strict version/method/profile request selection, without fallback.
use super::{DESCRIBE, PROFILE, VERSION, reviewed};
use crate::{
    files::{Error, Packet},
    services::Method,
};

pub fn request(method: Method, context: u32) -> Result<Packet, Error> {
    reviewed::digest(method)?;
    let mut p = Packet::new(DESCRIBE);
    p.id = method as u32;
    p.version = VERSION;
    p.arg = PROFILE;
    p.context = context;
    Ok(p)
}

pub fn decode_request(p: &Packet) -> Result<Method, Error> {
    if p.op != DESCRIBE || p.status != 0 || p.count != 0 || p.data != [0; 40] {
        return Err(Error::Protocol);
    }
    if p.version != VERSION || p.arg != PROFILE {
        return Err(Error::UnsupportedVersion);
    }
    method(p.id)
}

pub(super) fn method(id: u32) -> Result<Method, Error> {
    match id {
        5 => Ok(Method::OperationsGet),
        6 => Ok(Method::OperationsCancel),
        _ => Err(Error::Unsupported),
    }
}
