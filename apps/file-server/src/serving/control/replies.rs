// SPDX-License-Identifier: Apache-2.0
// Pending transport replies lose all result data when their binding is fenced.
use rustic_file_service::Grant;
use rustic_sdk::{
    abi::files::{Error, Packet},
    ipc::Message,
};

pub(in super::super) fn denial(grant: &Grant, now: u64) -> Option<Error> {
    if grant.rights == 0 {
        Some(Error::Revoked)
    } else if grant.expires != 0 && now >= grant.expires {
        Some(Error::Expired)
    } else {
        None
    }
}

pub(super) fn refuse(request: &Packet, error: Error) -> Packet {
    let mut reply = Packet::new(request.op);
    reply.context = request.context;
    reply.status = error as u8;
    reply
}

// Keep temporary packet/IPC buffers out of the publication's polling frame.
#[inline(never)]
pub(in super::super) fn restrict(reply: &mut Option<Message>, error: Option<Error>) {
    let (Some(message), Some(error)) = (reply.as_ref(), error) else {
        return;
    };
    let Ok(packet) = Packet::decode(message.payload()) else {
        *reply = None;
        return;
    };
    *reply = Some(Message::new(message.correlation(), &refuse(&packet, error).encode()).unwrap());
}
