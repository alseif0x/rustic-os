// SPDX-License-Identifier: Apache-2.0
//! Execute one scheduled admission or publish its durable no-effect prevention.
use super::{Candidate7, Ticket7};
use crate::{
    reply,
    v7::{
        Control7, Server7,
        control::{self, Owner},
    },
};
use rustic_abi::files::{Error, INSPECT_RIGHT, WRITE_RIGHT};
use rustic_fs::{
    Disk, PollDisk7, PreventionReason,
    format7::{Record7, RecordState},
};

impl Server7<'_> {
    /// Whether this server incarnation has a volatile lifecycle ticket waiting.
    pub fn has_scheduled(&self) -> bool {
        self.scheduler.has_scheduled()
    }

    /// Drive at most one volatile lifecycle ticket. `true` means one ticket was
    /// processed; it does not mean that execution committed. Query the durable
    /// admission status for that result. Restarting the server starts empty.
    pub fn run_scheduled<D: Disk + PollDisk7>(
        &mut self,
        disk: &mut D,
        now: u64,
        mut control_callback: impl FnMut(&mut Control7<'_>) -> u64,
    ) -> bool {
        if !self.scheduler.has_scheduled() {
            return false;
        }
        if self.scheduler.refresh(self.volume, &self.grants).is_err() {
            self.scheduler.clear();
            if self.volume.header().is_err() {
                self.writes.forget_receipts();
            }
            return true;
        }
        let Some(ticket) = self.scheduler.front() else {
            return false;
        };
        let Some(candidate) = self.scheduler.candidate(ticket.id) else {
            self.scheduler.finish();
            return true;
        };
        if candidate.record.subject != ticket.subject
            || candidate.record.state != RecordState::Admitted
        {
            self.scheduler.finish();
            return true;
        }

        let initial = initial_prevention(ticket, candidate, &self.grants, now);
        let caller = super::super::super::control::Caller7 {
            slot: ticket.caller.slot,
            peer: ticket.caller.peer,
            context: ticket.caller.context,
        };
        let mut transfers = 0;
        for index in 0..crate::v7::CLIENTS7 {
            if self.writes.transfer_open(index) || self.plain.transfer_open(index) {
                transfers |= 1 << index;
            }
        }
        let mut owner = Owner::new(
            &mut self.grants,
            &mut self.scheduler,
            caller,
            transfers,
            &mut control_callback,
        );
        let result = match owner.activate(self.volume, candidate.record) {
            Ok(()) => run(self.volume, disk, candidate, initial, &mut owner),
            Err(error) => Err(error),
        };
        let lost = owner.lost();
        self.reset_lost(lost);
        if matches!(result, Err(Error::Uncertain)) || self.volume.header().is_err() {
            self.scheduler.clear();
            self.writes.forget_receipts();
        } else {
            self.scheduler.finish();
        }
        true
    }
}

pub(in crate::v7) fn run<D: Disk + PollDisk7>(
    volume: &mut rustic_fs::Volume7,
    disk: &mut D,
    candidate: Candidate7,
    initial_prevention: Option<PreventionReason>,
    owner: &mut Owner<'_>,
) -> Result<(), Error> {
    if let Some(reason) = initial_prevention {
        return prevent(volume, disk, &candidate.record, reason, owner);
    }

    let execution = match volume.prepare_execute(
        disk,
        super::super::records::identity(&candidate.record),
        candidate.record.previous,
    ) {
        Ok(publication) => control::drive(owner, publication, Some(INSPECT_RIGHT | WRITE_RIGHT)),
        Err(error) => Err(reply::error(error)),
    };
    let driven = match execution {
        Ok(driven) => driven,
        Err(Error::Version) => {
            return prevent(
                volume,
                disk,
                &candidate.record,
                PreventionReason::VersionConflict,
                owner,
            );
        }
        Err(Error::NotFound | Error::IsDirectory) => {
            return prevent(
                volume,
                disk,
                &candidate.record,
                PreventionReason::AuthorityLost,
                owner,
            );
        }
        Err(error) => return Err(error),
    };

    if driven.record.is_some() {
        // Once the header might have been submitted, stopping is too late; the
        // publication's durable result wins and no second prevention is made.
        return Ok(());
    }
    prevent(
        volume,
        disk,
        &candidate.record,
        if driven.denied.is_some() {
            PreventionReason::AuthorityLost
        } else {
            PreventionReason::Requested
        },
        owner,
    )
}

fn prevent<D: Disk + PollDisk7>(
    volume: &mut rustic_fs::Volume7,
    disk: &mut D,
    record: &Record7,
    reason: PreventionReason,
    owner: &mut Owner<'_>,
) -> Result<(), Error> {
    owner.stop_active();
    owner.set_cleanup(true);
    let publication = volume
        .prepare_cancellation(
            disk,
            super::super::records::identity(record),
            record.previous,
            reason,
        )
        .map_err(reply::error)?;
    let driven = control::drive(owner, publication, None)?;
    match driven.record {
        Some(record) if record.state == RecordState::Cancelled => Ok(()),
        _ => Err(Error::Uncertain),
    }
}

pub(in crate::v7) fn initial_prevention(
    ticket: Ticket7,
    candidate: Candidate7,
    grants: &super::super::super::grants::Grants,
    now: u64,
) -> Option<PreventionReason> {
    use super::TicketKind7;
    if ticket.kind == TicketKind7::Prevent {
        return Some(PreventionReason::Requested);
    }
    let live = candidate
        .scope
        .check(
            grants,
            ticket.caller.slot,
            ticket.caller.peer,
            ticket.caller.context,
            now,
            INSPECT_RIGHT | WRITE_RIGHT,
        )
        .is_ok_and(|grant| grant == ticket.saved);
    if !live {
        Some(PreventionReason::AuthorityLost)
    } else if ticket.stop {
        Some(PreventionReason::Requested)
    } else {
        None
    }
}
