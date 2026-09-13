// SPDX-License-Identifier: Apache-2.0
//! One owner-controlled durable intent, with version-checked replacement.
use super::{Error, Session};
use rustic_sdk::files::{self, Metadata, read, reference::Version};
use rustic_shell::task_intent::Intent;

const NAME: &str = "tasks-intent";

pub(super) fn locate(session: &mut Session) -> Result<Option<Metadata>, Error> {
    let config = session.files.resolve(0, "/config")?;
    match session.files.lookup(config, NAME) {
        Ok(metadata) if !metadata.directory => Ok(Some(metadata)),
        Ok(_) => Err(Error::TaskJournal),
        Err(files::Error::NotFound) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn idle(session: &mut Session) -> Result<(), Error> {
    if let Some(metadata) = locate(session)?
        && metadata.length != 0
    {
        return Err(Error::TaskPending(metadata.version));
    }
    Ok(())
}

pub(super) fn retain(
    session: &mut Session,
    intent: &Intent,
    _lose_reply: bool,
) -> Result<Metadata, Error> {
    let original = match locate(session)? {
        Some(metadata) if metadata.length == 0 => metadata,
        Some(metadata) => return Err(Error::TaskPending(metadata.version)),
        None => {
            let config = session.files.resolve(0, "/config")?;
            session.files.create(config, NAME, false)?
        }
    };
    #[cfg(feature = "tasks-acceptance")]
    if _lose_reply {
        super::acceptance::discard_journal(session, original, intent.encoded())?;
        return Err(files::Error::Uncertain.into());
    }
    // A lost journal acknowledgement never permits target submission. A later
    // recovery only queries: it cannot infer whether this process reached it.
    let saved = session
        .files
        .replace(original.id, original.version, intent.encoded())?;
    let (observed, restored) = load(session)?.ok_or(Error::TaskJournal)?;
    if observed.id != saved.id
        || observed.version != saved.version
        || restored.encoded() != intent.encoded()
    {
        return Err(Error::TaskJournal);
    }
    let request = intent.request(saved.version)?;
    let refs = session.files.references(saved.space.into(), saved.id)?;
    if refs.workspace.lineage() != request.workspace.lineage() {
        return Err(Error::TaskJournal);
    }
    Ok(saved)
}

pub(super) fn load(session: &mut Session) -> Result<Option<(Metadata, Intent)>, Error> {
    let Some(metadata) = locate(session)? else {
        return Ok(None);
    };
    if metadata.length == 0 {
        return Ok(None);
    }
    let mut bytes = [0; 1024];
    let info = snapshot(session, metadata, &mut bytes)?;
    let intent = Intent::decode(&bytes[..info.length]).map_err(|_| Error::TaskJournal)?;
    if intent.request(metadata.version)?.workspace.lineage() != info.references.workspace.lineage()
    {
        return Err(Error::TaskJournal);
    }
    Ok(Some((metadata, intent)))
}

pub(super) fn clear(session: &mut Session, metadata: Metadata) -> Result<(), Error> {
    // Keep a zero-length idle record. Unlike REMOVE, replace enforces the saved
    // version, so cleanup cannot delete a newer owner's intent by object ID.
    session.files.replace(metadata.id, metadata.version, &[])?;
    Ok(())
}

pub(super) fn snapshot(
    session: &mut Session,
    metadata: Metadata,
    bytes: &mut [u8; 1024],
) -> Result<read::Info, Error> {
    if metadata.directory {
        return Err(files::Error::IsDirectory.into());
    }
    let refs = session
        .files
        .references(metadata.space.into(), metadata.id)?;
    let info = session.files.read_range(
        read::Request {
            workspace: refs.workspace,
            resource: refs.resource,
            expected_version: Some(Version::new(metadata.version)?),
            offset: 0,
            length: 1024,
        },
        bytes,
    )?;
    if !info.eof || info.offset != 0 || info.size as usize != info.length {
        return Err(files::Error::Protocol.into());
    }
    Ok(info)
}
