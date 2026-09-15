// SPDX-License-Identifier: Apache-2.0
//! Explicit native failure cuts; absent from ordinary builds.
use crate::{Authority, Error, Note, Report};
use rustic_sdk::{
    abi::files::{Packet, REPLACE_COMMIT, operation::Replacement},
    files,
    ipc::{Endpoint, Message},
    runtime,
};

pub(crate) fn discard_reply<A: Authority, R: Report>(
    authority: &mut A,
    report: &mut R,
    request: Replacement,
    bytes: &[u8],
) -> Result<(), Error> {
    authority.files().stage_replace(request, bytes)?;
    let mut packet = Packet::new(REPLACE_COMMIT);
    packet.id = request.resource.object();
    discard(authority, report, packet)
}

pub(crate) fn discard_journal<A: Authority, R: Report>(
    authority: &mut A,
    report: &mut R,
    original: files::Metadata,
    bytes: &[u8],
) -> Result<(), Error> {
    use rustic_sdk::abi::files::{BEGIN, CHUNK, COMMIT, DATA};
    let mut packet = Packet::new(BEGIN);
    packet.id = original.id;
    packet.version = original.version;
    packet.arg = bytes.len() as u32;
    authority.files().request(packet)?;
    for (index, chunk) in bytes.chunks(DATA).enumerate() {
        let mut packet = Packet::new(CHUNK);
        packet.id = original.id;
        packet.arg = (index * DATA) as u32;
        packet.count = chunk.len() as u8;
        packet.data[..chunk.len()].copy_from_slice(chunk);
        authority.files().request(packet)?;
    }
    let mut packet = Packet::new(COMMIT);
    packet.id = original.id;
    discard(authority, report, packet)
}

fn discard<A: Authority, R: Report>(
    authority: &mut A,
    report: &mut R,
    mut packet: Packet,
) -> Result<(), Error> {
    packet.context = authority.files().context;
    let endpoint = Endpoint::from_bootstrap(authority.files().token());
    let result = (|| {
        endpoint
            .send(&Message::new(u64::MAX, &packet.encode()).map_err(|_| files::Error::Protocol)?)
            .map_err(|_| files::Error::Uncertain)?;
        let deadline = runtime::clock().saturating_add(1100);
        loop {
            match endpoint.receive() {
                // Discard the real message before packet/receipt decoding. A
                // received error would be discarded too; readiness is no proof.
                Ok(_) => break,
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock))
                    if runtime::clock() < deadline =>
                {
                    runtime::wait_set(&[endpoint.token()], 1)
                        .map_err(|_| files::Error::Uncertain)?;
                }
                _ => return Err(files::Error::Uncertain),
            }
        }
        report.note(Note::ReplyDiscarded);
        Ok(())
    })();
    let _ = endpoint.close();
    authority.files().rebind(0, 0, 0);
    result?;
    Err(files::Error::Uncertain.into())
}
