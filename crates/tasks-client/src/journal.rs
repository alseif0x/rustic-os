// SPDX-License-Identifier: Apache-2.0
//! One owner-controlled durable intent, with version-checked replacement.
use crate::{Authority, Error, Pending, Record, Report, intent::Intent, record::Place};
use rustic_sdk::files::{self, Metadata, read, reference::Version};

pub(crate) fn locate<A: Authority>(
    authority: &mut A,
    record: &Record<'_>,
) -> Result<Option<Metadata>, Error> {
    match record.place() {
        Place::Named { directory, name } => {
            let directory = authority.files().resolve(0, directory)?;
            match authority.files().lookup(directory, name) {
                Ok(metadata) if !metadata.directory => Ok(Some(metadata)),
                Ok(_) => Err(Error::Journal),
                Err(files::Error::NotFound) => Ok(None),
                Err(error) => Err(error.into()),
            }
        }
        // A granted object is never created here: an absent or unreadable grant
        // is a refusal, not an idle record, so the refusal is forwarded.
        Place::Granted(id) => match authority.files().stat(id) {
            Ok(metadata) if !metadata.directory => Ok(Some(metadata)),
            Ok(_) => Err(Error::Journal),
            Err(error) => Err(error.into()),
        },
        Place::Invalid => Err(Error::Journal),
    }
}

pub(crate) fn pending<A: Authority>(
    authority: &mut A,
    record: &Record<'_>,
) -> Result<Pending, Error> {
    Ok(match locate(authority, record)? {
        Some(metadata) => Pending::of(metadata.length, metadata.version),
        None => Pending::Idle,
    })
}

pub(crate) fn idle<A: Authority>(authority: &mut A, record: &Record<'_>) -> Result<(), Error> {
    pending(authority, record)?.idle()
}

pub(crate) fn retain<A: Authority, R: Report>(
    authority: &mut A,
    _report: &mut R,
    record: &Record<'_>,
    intent: &Intent,
    _lose_reply: bool,
) -> Result<Metadata, Error> {
    let original = match locate(authority, record)? {
        Some(metadata) if metadata.length == 0 => metadata,
        Some(metadata) => return Err(Error::Pending(metadata.version)),
        None => {
            let (directory, name) = record.creation()?;
            let directory = authority.files().resolve(0, directory)?;
            authority.files().create(directory, name, false)?
        }
    };
    #[cfg(feature = "tasks-acceptance")]
    if _lose_reply {
        crate::acceptance::discard_journal(authority, _report, original, intent.encoded())?;
        return Err(files::Error::Uncertain.into());
    }
    // A lost journal acknowledgement never permits target submission. A later
    // recovery only queries: it cannot infer whether this process reached it.
    let saved = authority
        .files()
        .replace(original.id, original.version, intent.encoded())?;
    let (observed, restored) = load(authority, record)?.ok_or(Error::Journal)?;
    if observed.id != saved.id
        || observed.version != saved.version
        || restored.encoded() != intent.encoded()
    {
        return Err(Error::Journal);
    }
    let request = intent.request(saved.version)?;
    let refs = authority.files().references(saved.space.into(), saved.id)?;
    if refs.workspace.lineage() != request.workspace.lineage() {
        return Err(Error::Journal);
    }
    Ok(saved)
}

pub(crate) fn load<A: Authority>(
    authority: &mut A,
    record: &Record<'_>,
) -> Result<Option<(Metadata, Intent)>, Error> {
    let Some(metadata) = locate(authority, record)? else {
        return Ok(None);
    };
    if metadata.length == 0 {
        return Ok(None);
    }
    let mut bytes = [0; 1024];
    let info = snapshot(authority, metadata, &mut bytes)?;
    let intent = Intent::decode(&bytes[..info.length]).map_err(|_| Error::Journal)?;
    if intent.request(metadata.version)?.workspace.lineage() != info.references.workspace.lineage()
    {
        return Err(Error::Journal);
    }
    Ok(Some((metadata, intent)))
}

pub(crate) fn clear<A: Authority>(authority: &mut A, metadata: Metadata) -> Result<(), Error> {
    // Keep a zero-length idle record. Unlike REMOVE, replace enforces the saved
    // version, so cleanup cannot delete a newer owner's intent by object ID.
    authority
        .files()
        .replace(metadata.id, metadata.version, &[])?;
    Ok(())
}

pub(crate) fn snapshot<A: Authority>(
    authority: &mut A,
    metadata: Metadata,
    bytes: &mut [u8; 1024],
) -> Result<read::Info, Error> {
    if metadata.directory {
        return Err(files::Error::IsDirectory.into());
    }
    let refs = authority
        .files()
        .references(metadata.space.into(), metadata.id)?;
    let info = authority.files().read_range(
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

/// Deliberately discards the retained record. It does not cancel or undo a
/// submitted effect, so the caller must name the exact retained version.
pub(crate) fn forget<A: Authority>(
    authority: &mut A,
    record: &Record<'_>,
    key: u64,
) -> Result<(), Error> {
    let (saved, _) = load(authority, record)?.ok_or(Error::Journal)?;
    if saved.version != key {
        return Err(Error::Pending(saved.version));
    }
    clear(authority, saved)
}
