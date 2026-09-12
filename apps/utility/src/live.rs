// SPDX-License-Identifier: Apache-2.0
//! Owner-launched fault fixture: leave a scheduling/stop/result reply unread.
use rustic_sdk::{
    abi::files::{Error, admission as a, lifecycle},
    files::Client,
    ipc::{Endpoint, Message},
    runtime,
};

/// Send one schedule/stop/query and return without receiving. The queued reply stays
/// undelivered until this client drains it, which never makes it a durable result.
pub(super) fn discard_admission_reply(
    files: &Client,
    id: a::AdmissionId,
    op: u8,
) -> Result<(), Error> {
    if !matches!(
        op,
        a::SCHEDULE | a::REQUEST_CANCEL | a::GET | lifecycle::CANCEL
    ) {
        return Err(Error::Invalid);
    }
    let packet = if op == lifecycle::CANCEL {
        lifecycle::CancelAck::request(id, files.context)?
    } else {
        id.packet(op, files.context)?
    };
    let message = Message::new(u64::MAX, &packet.encode()).map_err(|_| Error::Protocol)?;
    let endpoint = Endpoint::from_bootstrap(files.token());
    endpoint.send(&message).map_err(|_| Error::Uncertain)?;
    if op == a::GET {
        // The result-loss fixture starts after settlement. Prove an unread reply
        // is ready, with a bounded wait; readiness says nothing about its status.
        runtime::wait_set(&[endpoint.token()], 100).map_err(|_| Error::Uncertain)?;
    }
    Ok(())
}
