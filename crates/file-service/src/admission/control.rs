// SPDX-License-Identifier: Apache-2.0
//! One owner-control loop for admission, file effects and terminal housekeeping.
use super::Caller;
use crate::{Clients, reply};
use core::task::Poll;
use rustic_abi::files::Error;
use rustic_fs::{PollDisk, Publication, PublicationPhase as Phase};

pub(super) struct Settled<T> {
    pub(super) result: Option<T>,
    pub(super) denied: Option<Error>,
}

/// None is service-owned terminal housekeeping after lost execution authority.
/// It must settle even if the original client disappears; it cannot execute data.
pub(super) fn drive<D: PollDisk, T: Copy>(
    clients: &mut Clients,
    mut write: Publication<'_, D, T>,
    caller: Option<Caller>,
    control: &mut impl FnMut(&mut Clients, bool) -> u64,
) -> Result<Settled<T>, Error> {
    let mut denied = None;
    loop {
        let now = control(clients, write.pending());
        clients.expire(now);
        // Grants can only be revoked/detached/expired through this borrow.
        // Issuance and changes of scope/rights/subject require the whole Server.
        if let Some(caller) = caller
            && let Err(error) = caller.check(clients, now)
        {
            denied.get_or_insert(error);
            write.cancel().map_err(reply::error)?;
        }
        if matches!(write.phase(), Phase::Committed | Phase::Cancelled) {
            return Ok(Settled {
                result: write.result(),
                denied,
            });
        }
        if let Poll::Ready(result) = write.poll_advance() {
            result.map_err(reply::error)?;
        }
    }
}
