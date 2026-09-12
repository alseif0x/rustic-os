// SPDX-License-Identifier: Apache-2.0
//! Owner-launched fault fixture: submit a live stop and drop its acknowledgement.
use rustic_sdk::{
    abi::files::{Error, admission as a},
    files::Client,
    ipc::{Endpoint, Message},
};

/// Send one REQUEST_CANCEL and return without receiving. The queued reply stays
/// undelivered until this client drains it, which never makes it a durable result.
pub(super) fn discard_stop_reply(files: &Client, id: a::AdmissionId) -> Result<(), Error> {
    let packet = id.packet(a::REQUEST_CANCEL, files.context)?;
    let message = Message::new(u64::MAX, &packet.encode()).map_err(|_| Error::Protocol)?;
    Endpoint::from_bootstrap(files.token())
        .send(&message)
        .map_err(|_| Error::Uncertain)
}
