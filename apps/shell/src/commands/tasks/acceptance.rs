// SPDX-License-Identifier: Apache-2.0
//! Explicit native failure cuts; absent from ordinary builds.
use super::{Error, Session, argument, exact, mutation, output, parse_edit};
use rustic_sdk::{
    abi::files::{Packet, REPLACE_COMMIT, operation::Replacement},
    files,
    ipc::{Endpoint, Message},
    runtime,
};

pub(in crate::commands) fn execute(
    session: &mut Session,
    args: &rustic_shell::parser::Args<'_>,
) -> Result<(), Error> {
    exact(args, 5)?;
    let cut = match argument(args, 1)? {
        "prepared" => mutation::Cut::Prepared,
        "conflict" => mutation::Cut::HumanConflict,
        "lost-reply" => mutation::Cut::LostReply,
        "lost-journal" => mutation::Cut::LostJournal,
        _ => return Err(Error::Usage),
    };
    let edit = parse_edit(argument(args, 2)?, argument(args, 4)?)?;
    mutation::apply_cut(session, argument(args, 3)?, edit, cut)
}

pub(super) fn discard_reply(
    session: &mut Session,
    request: Replacement,
    bytes: &[u8],
) -> Result<(), Error> {
    session.files.stage_replace(request, bytes)?;
    let mut packet = Packet::new(REPLACE_COMMIT);
    packet.id = request.resource.object();
    discard(session, packet)
}

pub(super) fn discard_journal(
    session: &mut Session,
    original: files::Metadata,
    bytes: &[u8],
) -> Result<(), Error> {
    use rustic_sdk::abi::files::{BEGIN, CHUNK, COMMIT, DATA};
    let mut packet = Packet::new(BEGIN);
    packet.id = original.id;
    packet.version = original.version;
    packet.arg = bytes.len() as u32;
    session.files.request(packet)?;
    for (index, chunk) in bytes.chunks(DATA).enumerate() {
        let mut packet = Packet::new(CHUNK);
        packet.id = original.id;
        packet.arg = (index * DATA) as u32;
        packet.count = chunk.len() as u8;
        packet.data[..chunk.len()].copy_from_slice(chunk);
        session.files.request(packet)?;
    }
    let mut packet = Packet::new(COMMIT);
    packet.id = original.id;
    discard(session, packet)
}

fn discard(session: &mut Session, mut packet: Packet) -> Result<(), Error> {
    packet.context = session.files.context;
    let endpoint = Endpoint::from_bootstrap(session.files.token());
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
        output::text("Task acceptance: real commit reply discarded without decoding\r\n");
        Ok(())
    })();
    let _ = endpoint.close();
    session.files.rebind(0, 0, 0);
    result?;
    Err(files::Error::Uncertain.into())
}
