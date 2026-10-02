// SPDX-License-Identifier: Apache-2.0
//! Private owner authority and V7 lifecycle dispatch.
use super::Replies;
use crate::disk::Disk;
use rustic_file_service::{CLIENTS7, Control7, Grant7, GrantRequest7, Server7};
use rustic_sdk::{abi::files, ipc::Endpoint};

pub(super) type Bindings = [Option<Grant7>; CLIENTS7];

pub(super) fn request(
    server: &mut Server7<'_>,
    disk: &mut Disk,
    replies: &mut Replies,
    words: [u64; 8],
) -> [u64; 8] {
    let mut result = [0; 8];
    let parsed = (|| {
        let slot = usize::try_from(words[1]).map_err(|_| files::Error::Invalid)?;
        match words[0] {
            command if command == u64::from(files::GRANT) => {
                let before = bindings(server);
                let grant = server.grant(slot, root_request(words)?)?;
                let after = bindings(server);
                cleanup_replaced(server, replies, before, after);
                result[1] = u64::from(grant.context);
            }
            command if command == u64::from(files::GRANT_SECOND_SCOPE) => {
                if words[4..].iter().any(|word| *word != 0) {
                    return Err(files::Error::Protocol);
                }
                result[1] = u64::from(server.extend(
                    slot,
                    u32::try_from(words[2]).map_err(|_| files::Error::Invalid)?,
                    u32::try_from(words[3]).map_err(|_| files::Error::Invalid)?,
                )?);
            }
            command if command == u64::from(files::REVOKE) => {
                if words[2..].iter().any(|word| *word != 0) {
                    return Err(files::Error::Protocol);
                }
                let before_count = server.pending();
                let before = bindings(server);
                let lost = server.revoke(slot)?;
                let after_count = server.pending();
                let after = bindings(server);
                clear_lost(&before, &after, lost, replies);
                result[1] = u64::from(lost);
                result[2] = before_count.saturating_sub(after_count) as u64;
                settlement_state(server, &mut result);
            }
            40 if slot != 0 && words[2..].iter().all(|word| *word == 0) => {
                let root = u32::try_from(words[1]).map_err(|_| files::Error::Invalid)?;
                let before_count = server.pending();
                let before = bindings(server);
                let lost = server.revoke_root(root);
                let after_count = server.pending();
                let after = bindings(server);
                clear_lost(&before, &after, lost, replies);
                result[1] = u64::from(lost);
                result[2] = before_count.saturating_sub(after_count) as u64;
                settlement_state(server, &mut result);
            }
            37 => {
                let before = bindings(server);
                let grant = server.derive(
                    slot,
                    u32::try_from(words[2]).map_err(|_| files::Error::Invalid)?,
                    GrantRequest7 {
                        peer: words[3],
                        endpoint: words[4],
                        scope: u32::try_from(words[5]).map_err(|_| files::Error::Invalid)?,
                        rights: u8::try_from(words[6]).map_err(|_| files::Error::Invalid)?,
                        subject: 0,
                        expires: words[7],
                    },
                    rustic_sdk::runtime::clock(),
                )?;
                let after = bindings(server);
                cleanup_replaced(server, replies, before, after);
                result[1] = u64::from(grant.context);
            }
            command
                if command == u64::from(files::STATUS)
                    && words[1..].iter().all(|word| *word == 0) =>
            {
                result[1] = server.pending() as u64;
            }
            35 if slot < CLIENTS7 && words[2..].iter().all(|word| *word == 0) => {
                detach_slot(server, replies, slot);
            }
            command
                if command == u64::from(files::MAINTAIN_RETENTION)
                    && words[1..].iter().all(|word| *word == 0) =>
            {
                let done = server.maintain_retention(disk)?;
                result[1] = done.previous_epoch;
                result[2] = done.epoch;
                result[3] = u64::from(done.records);
                result[4] = u64::from(done.sectors);
            }
            _ => return Err(files::Error::Protocol),
        }
        Ok(())
    })();
    if let Err(error) = parsed {
        result[0] = error as u64;
    }
    result
}

fn root_request(words: [u64; 8]) -> Result<GrantRequest7, files::Error> {
    Ok(GrantRequest7 {
        peer: words[2],
        endpoint: words[3],
        scope: u32::try_from(words[4]).map_err(|_| files::Error::Invalid)?,
        rights: u8::try_from(words[5]).map_err(|_| files::Error::Invalid)?,
        subject: words[7],
        expires: words[6],
    })
}

fn settlement_state(server: &Server7<'_>, words: &mut [u64; 8]) {
    let header = server.volume().header();
    words[3] = u64::from(header.is_err());
    // V7 clears the in-memory header when fenced, so zero here means that the
    // final sequence is unavailable; the fenced bit carries that distinction.
    words[4] = header.map_or(0, |header| header.sequence);
}

pub(super) fn detach_slot(server: &mut Server7<'_>, replies: &mut Replies, slot: usize) -> u8 {
    let before = bindings(server);
    let lost = server.detach(slot);
    let after = bindings(server);
    clear_lost(&before, &after, lost, replies);
    lost
}

pub(super) fn discard_revoked_slot(server: &Server7<'_>, replies: &mut Replies, slot: usize) {
    replies[slot] = None;
    if let Some(grant) = server.grant_at(slot)
        && grant.rights == 0
    {
        close_unless_reused(grant.endpoint, &bindings(server));
    }
}

fn bindings(server: &Server7<'_>) -> Bindings {
    core::array::from_fn(|slot| server.grant_at(slot))
}

pub(super) fn control_bindings(control: &Control7<'_>) -> Bindings {
    core::array::from_fn(|slot| control.grant_at(slot))
}

/// Slots whose old binding was removed, replaced or newly fenced by install.
fn replaced(before: &Bindings, after: &Bindings) -> u8 {
    let mut lost = 0;
    for slot in 0..CLIENTS7 {
        if let Some(old) = before[slot]
            && after[slot].is_none_or(|current| {
                current.context != old.context || old.rights != 0 && current.rights == 0
            })
        {
            lost |= 1 << slot;
        }
    }
    lost
}

fn cleanup_replaced(
    server: &mut Server7<'_>,
    replies: &mut Replies,
    before: Bindings,
    after: Bindings,
) {
    let lost = replaced(&before, &after);
    clear_lost(&before, &after, lost, replies);
    // A successfully replaced root fences its old helper bindings while
    // retaining their endpoint/context snapshots for this cleanup. Forget only
    // those old fenced slots; the new binding at the install target survives.
    for slot in 0..CLIENTS7 {
        if lost & (1 << slot) != 0
            && let (Some(old), Some(current)) = (before[slot], after[slot])
            && old.context == current.context
            && old.rights != 0
            && current.rights == 0
        {
            server.detach(slot);
        }
    }
}

fn clear_lost(before: &Bindings, after: &Bindings, lost: u8, replies: &mut Replies) {
    for slot in 0..CLIENTS7 {
        if lost & (1 << slot) == 0 {
            continue;
        }
        replies[slot] = None;
        if let Some(old) = before[slot] {
            close_unless_reused(old.endpoint, after);
        }
    }
}

fn close_unless_reused(endpoint: u64, after: &Bindings) {
    if !after
        .iter()
        .flatten()
        .any(|grant| grant.rights != 0 && grant.endpoint == endpoint)
    {
        let _ = Endpoint::from_bootstrap(endpoint).close();
    }
}

pub(super) fn cleanup_control(
    control: &Control7<'_>,
    replies: &mut Replies,
    before: Bindings,
    lost: u8,
) {
    let after = control_bindings(control);
    clear_lost(&before, &after, lost, replies);
}

pub(super) fn discard_revoked_control_slot(
    control: &Control7<'_>,
    replies: &mut Replies,
    slot: usize,
) {
    replies[slot] = None;
    if let Some(grant) = control.grant_at(slot)
        && grant.rights == 0
    {
        close_unless_reused(grant.endpoint, &control_bindings(control));
    }
}

pub(super) fn detach_control(control: &mut Control7<'_>, replies: &mut Replies, slot: usize) -> u8 {
    let before = control_bindings(control);
    let lost = control.detach(slot);
    cleanup_control(control, replies, before, lost);
    lost
}
