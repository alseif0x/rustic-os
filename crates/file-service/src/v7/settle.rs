// SPDX-License-Identifier: Apache-2.0
//! Drive one V7 pollable publication to settlement inside the current request.
//!
//! The service has no owner-control loop around V7 publications yet: a request
//! that starts one (admission acceptance, execution, cancellation) polls it
//! until it settles, and no other request is served meanwhile.
//!
//! The driver only accepts publications over the [`Synchronous`] adapter,
//! whose every read, write and flush settles inside its poll. Each
//! `poll_advance` therefore completes one command and moves the publication
//! forward, so the loop ends after the bounded command sequence of one
//! publication and never spins on `Pending`. If a poll ever reported `Pending`
//! anyway, the driver stops and reports `Uncertain`; dropping a publication
//! with a command outstanding keeps the volume fenced.
use crate::{disk::Synchronous, reply};
use core::task::Poll;
use rustic_abi::files::Error;
use rustic_fs::{Disk, PollPublication7, Publication7Phase, format7::Record7};

/// The record the publication made durable (or the retained record a settled
/// replay reports).
///
/// A disk failure during publication is [`Error::Uncertain`] (the volume stays
/// fenced until a remount selects the durable generation); any other refusal
/// carries the volume's error. A publication that ends cancelled was never
/// asked to stop, so it is reported as `Uncertain` rather than as no effect.
pub(super) fn settle<D: Disk>(
    mut publication: PollPublication7<'_, Synchronous<'_, D>>,
) -> Result<Record7, Error> {
    loop {
        match publication.poll_advance() {
            Poll::Ready(Ok(Publication7Phase::Committed)) => {
                return publication.result().ok_or(Error::Uncertain);
            }
            Poll::Ready(Ok(Publication7Phase::Cancelled)) => return Err(Error::Uncertain),
            Poll::Ready(Ok(_)) => {}
            // Unreachable over the synchronous adapter; never busy-wait.
            Poll::Pending => return Err(Error::Uncertain),
            Poll::Ready(Err(error)) => return Err(reply::error(error)),
        }
    }
}
