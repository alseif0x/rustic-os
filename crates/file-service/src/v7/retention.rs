// SPDX-License-Identifier: Apache-2.0
//! Owner-requested retention maintenance of the served V7 volume.
//!
//! Policy lives here: maintenance runs only when the owner asks for it through
//! the administrative channel, never to make room for a write, and it is
//! refused while any client transfer is open, so a retry in flight never loses
//! the record it replays. The volume refuses open stages and unresolved
//! admissions itself and performs the durable publication.
use crate::reply;
use rustic_abi::files::Error;
use rustic_fs::{Disk, Volume7, format7::RETAINED};

// The owner's reply bound in the ABI must be the volume's retained budget.
const _: () = assert!(RETAINED == rustic_abi::files::workspace::RETAINED_RECORDS as usize);

/// What one completed maintenance published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Maintenance7 {
    /// Retry epoch before the maintenance; retries naming it now expire.
    pub previous_epoch: u64,
    /// The newly published retry epoch that fresh writes must use.
    pub epoch: u64,
    /// Terminal retained records dropped.
    pub records: u32,
    /// Payload sectors returned to the free map: snapshot extents no live file
    /// owned.
    pub sectors: u32,
}

/// Run the maintenance when no transfer is open. A refusal before the volume
/// starts publishing leaves it unchanged; any error from inside the
/// publication (a disk failure is `Uncertain`) leaves it fenced. Once it has
/// published, a failure to describe the effect is `Uncertain`, never an error
/// that implies no effect.
pub(super) fn maintain(
    volume: &mut Volume7,
    disk: &mut impl Disk,
    transfers_open: bool,
) -> Result<Maintenance7, Error> {
    if transfers_open {
        return Err(Error::Busy);
    }
    let previous_epoch = volume.header().map_err(reply::error)?.epoch;
    let records = volume
        .retained_records()
        .map_err(reply::error)?
        .iter()
        .flatten()
        .count();
    let free_before = volume.free_sectors().map_err(reply::error)?;
    let epoch = volume.maintain_retention(disk).map_err(reply::error)?;
    // Published: the old epoch has expired whatever follows.
    let free_after = volume.free_sectors().map_err(|_| Error::Uncertain)?;
    let sectors = free_after
        .checked_sub(free_before)
        .and_then(|freed| u32::try_from(freed).ok())
        .ok_or(Error::Uncertain)?;
    let records = u32::try_from(records).map_err(|_| Error::Uncertain)?;
    if previous_epoch.checked_add(1) != Some(epoch) {
        return Err(Error::Uncertain);
    }
    Ok(Maintenance7 {
        previous_epoch,
        epoch,
        records,
        sectors,
    })
}
