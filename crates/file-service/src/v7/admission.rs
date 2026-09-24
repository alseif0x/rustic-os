// SPDX-License-Identifier: Apache-2.0
//! Profile-2 staged admission over the V7 service: a streamed admission stage
//! (OPEN, CHUNK, ACCEPT, ABORT), status by admission ID or retry identity (GET,
//! RETRY), retained observation (OBSERVE) and explicit durable execution or
//! cancellation (EXECUTE, CANCEL).
//!
//! Policy lives here, mirroring the v5 direct path:
//!
//! - Every step needs a nonzero retry subject. Staging and acceptance need
//!   write and inspection; status and observation need inspection; execution
//!   needs inspection and, for an admitted record, live write authority over
//!   the file; cancellation needs `CANCEL` and, because its reply discloses
//!   the admission's status, inspection.
//! - Only records of the grant's subject exist for it, and a record outside the
//!   grant scope is answered as missing.
//! - The admitting subject is the executor, under the authority it holds when
//!   it asks. A file version that changed since admission refuses execution
//!   with `Version` and leaves the record admitted; nothing converts it into a
//!   cancellation without an explicit CANCEL, whose retained cause is
//!   `Requested`.
//! - Execution or cancellation of a terminal record, and an exact retry of an
//!   admission, report the retained status without writing.
//!
//! - Authority lost while a publication is in flight (owner revocation,
//!   detach or expiry, mirroring the v5 owner-control loop): before the
//!   header, the publication stops with nothing durable; an execution is then
//!   recorded cancelled with cause `AuthorityLost`. After the header, the
//!   publication settles: a new admission is then cancelled with
//!   `AuthorityLost` (it can never execute), and a settled execution or
//!   cancellation stands and is reported `Uncertain`. Otherwise the caller
//!   receives its authority error.
//!
//! Each publication is driven to settlement inside its request, with owner
//! control between its polls; no other client request is served meanwhile.
//! The volume enforces versions, retry scopes, the retained-record budget and
//! publication barriers.
mod records;

use super::control::{Driven, Owner, drive};
use super::write::Writes;
use super::{Grant7, scope};
use crate::reply;
use rustic_abi::files::{
    admission::{self as a, AdmissionId},
    operation,
    workspace::{Lookup, Replacement},
    *,
};
use rustic_fs::{
    Disk, PollDisk7, PreventionReason, Stage7Kind, Volume7,
    format7::{Record7, RecordState},
};

/// Whether `p` selects the admission path. Profile-2 OPEN and RETRY carry the
/// explicit profile marker, so unmarked profile-1 requests stay `Unsupported`;
/// the other requests bind to an open admission transfer or name an admission
/// ID. Live scheduling and activity requests are not served on V7.
pub(super) fn selected(p: &Packet) -> bool {
    match p.op {
        a::OPEN => p.count == 40,
        a::RETRY => p.count == 28,
        a::CHUNK | a::ACCEPT | a::ABORT | a::GET | a::EXECUTE | a::CANCEL | a::OBSERVE => true,
        _ => false,
    }
}

pub(super) fn request<D: Disk + PollDisk7>(
    writes: &mut Writes,
    volume: &mut Volume7,
    disk: &mut D,
    slot: usize,
    grant: Grant7,
    p: Packet,
    owner: &mut Owner<'_>,
) -> Result<Packet, Error> {
    if grant.subject == 0 {
        return Err(Error::Denied);
    }
    const KIND: Stage7Kind = Stage7Kind::Admission;
    match p.op {
        a::OPEN => {
            let request = Replacement::decode(&p)?.request;
            writes.open(volume, slot, grant, request, p.arg, KIND)?;
            let mut ack = Packet::new(p.op);
            ack.context = p.context;
            Ok(ack)
        }
        a::CHUNK => {
            grant.holds(INSPECT_RIGHT)?;
            writes.chunk(volume, disk, slot, grant, KIND, p)
        }
        a::ABORT => {
            grant.holds(INSPECT_RIGHT)?;
            writes.abort(volume, slot, grant, KIND, p)
        }
        a::ACCEPT => {
            grant.holds(INSPECT_RIGHT)?;
            let transfer = writes.take_complete(slot, grant, KIND, &p)?;
            // Only an admission this request creates is the service's to
            // retire; an exact retry reports a record that already existed.
            let fresh = !retained(volume, grant.subject, transfer.request())?;
            let result = transfer.finish_admission(volume, disk, |publication| {
                drive(owner, publication, Some(INSPECT_RIGHT))
            });
            let driven = published(writes, volume, result)?;
            if let Some(error) = driven.denied {
                if fresh && let Some(record) = driven.record {
                    // Admitted after the caller lost its authority: no file
                    // effect started, so record why it will never run.
                    authority_lost(writes, volume, disk, owner, &record)?;
                }
                return Err(error);
            }
            let record = driven.record.ok_or(Error::Uncertain)?;
            // An admitted (or replayed) record is durable from here on.
            status_reply(volume, &record, &p).map_err(|_| Error::Uncertain)
        }
        a::GET => {
            grant.holds(INSPECT_RIGHT)?;
            let record = records::by_id(volume, grant, AdmissionId::decode(&p)?)?;
            status_reply(volume, &record, &p)
        }
        a::RETRY => {
            grant.holds(INSPECT_RIGHT)?;
            let mut lookup = p;
            lookup.op = OPERATION_RETRY;
            let operation::Lookup::Retry { workspace, retry } = Lookup::decode(&lookup)?.query
            else {
                return Err(Error::Protocol);
            };
            let record = records::by_retry(volume, grant, workspace, retry)?;
            status_reply(volume, &record, &p)
        }
        a::OBSERVE => {
            grant.holds(INSPECT_RIGHT)?;
            if !matches!(p.arg, a::OBSERVATION_VERSION | a::OBSERVATION_V2) {
                return Err(Error::UnsupportedVersion);
            }
            let record = records::by_id(volume, grant, AdmissionId::decode(&p)?)?;
            let lineage = volume.header().map_err(reply::error)?.lineage;
            crate::admission::observation_reply(records::observation(lineage, &record)?, p)
        }
        a::EXECUTE => {
            grant.holds(INSPECT_RIGHT)?;
            let record = records::by_id(volume, grant, AdmissionId::decode(&p)?)?;
            if record.state != RecordState::Admitted {
                return status_reply(volume, &record, &p);
            }
            // Execution is a file effect under live write authority.
            grant.holds(WRITE_RIGHT)?;
            scope::authorized_resource(volume, grant.scope, record.workspace, record.object)?;
            let result = volume
                .prepare_execute(disk, records::identity(&record), record.previous)
                .map_err(reply::error)
                .and_then(|publication| drive(owner, publication, Some(INSPECT_RIGHT)));
            let Driven {
                record: committed,
                denied,
            } = published(writes, volume, result)?;
            if committed.is_none() {
                // Stopped before its header: the file is unchanged. Record
                // the decisive cause so the admission can never run later.
                authority_lost(writes, volume, disk, owner, &record)?;
            }
            if let Some(error) = denied {
                // A settled effect stands, but it is not reported to a caller
                // whose authority is gone.
                return Err(if committed.is_some() {
                    Error::Uncertain
                } else {
                    error
                });
            }
            let committed = committed.ok_or(Error::Uncertain)?;
            status_reply(volume, &committed, &p).map_err(|_| Error::Uncertain)
        }
        a::CANCEL => {
            // The reply discloses the status, so cancelling also needs inspection.
            grant.holds(CANCEL_RIGHT | INSPECT_RIGHT)?;
            let record = records::by_id(volume, grant, AdmissionId::decode(&p)?)?;
            if record.state != RecordState::Admitted {
                // Committed means cancellation came too late.
                return status_reply(volume, &record, &p);
            }
            let result = volume
                .prepare_cancellation(
                    disk,
                    records::identity(&record),
                    record.previous,
                    PreventionReason::Requested,
                )
                .map_err(reply::error)
                .and_then(|publication| drive(owner, publication, Some(CANCEL_RIGHT)));
            let Driven {
                record: cancelled,
                denied,
            } = published(writes, volume, result)?;
            if let Some(error) = denied {
                // Stopped before its header, the admission stays admitted; a
                // settled cancellation stands but is not reported.
                return Err(if cancelled.is_some() {
                    Error::Uncertain
                } else {
                    error
                });
            }
            let cancelled = cancelled.ok_or(Error::Uncertain)?;
            status_reply(volume, &cancelled, &p).map_err(|_| Error::Uncertain)
        }
        _ => Err(Error::Unsupported),
    }
}

/// Whether `subject` already has a retained record under the request's retry
/// identity.
fn retained(
    volume: &Volume7,
    subject: u64,
    request: operation::Replacement,
) -> Result<bool, Error> {
    let workspace = request.workspace.root();
    let epoch = request.retry.epoch.value();
    let key = request.retry.key.value();
    Ok(volume
        .retained_records()
        .map_err(reply::error)?
        .iter()
        .flatten()
        .any(|record| {
            record.subject == subject
                && record.workspace == workspace
                && record.retry_epoch == epoch
                && record.retry_key == key
        }))
}

/// Service housekeeping after the caller lost its authority: durably cancel
/// the admitted `record` with cause `AuthorityLost`. It is driven with owner
/// control but nothing stops it; any failure is `Uncertain`, since the
/// caller's own publication already settled one way or the other.
fn authority_lost<D: Disk + PollDisk7>(
    writes: &mut Writes,
    volume: &mut Volume7,
    disk: &mut D,
    owner: &mut Owner<'_>,
    record: &Record7,
) -> Result<(), Error> {
    let result = volume
        .prepare_cancellation(
            disk,
            records::identity(record),
            record.previous,
            PreventionReason::AuthorityLost,
        )
        .map_err(reply::error)
        .and_then(|publication| drive(owner, publication, None));
    let driven = published(writes, volume, result).map_err(|_| Error::Uncertain)?;
    match driven.record {
        Some(record) if record.state == RecordState::Cancelled => Ok(()),
        _ => Err(Error::Uncertain),
    }
}

/// Pass a publication's result through, forgetting every cached receipt when
/// the volume ended fenced: they may name records the durable generation no
/// longer holds.
fn published<T>(
    writes: &mut Writes,
    volume: &Volume7,
    result: Result<T, Error>,
) -> Result<T, Error> {
    if volume.header().is_err() {
        writes.forget_receipts();
    }
    result
}

fn status_reply(volume: &Volume7, record: &Record7, p: &Packet) -> Result<Packet, Error> {
    let lineage = volume.header().map_err(reply::error)?.lineage;
    records::status(lineage, record)?.packet(p.op, p.context)
}

#[cfg(test)]
mod tests {
    use super::super::{control::Caller7, grants::Grants};
    use super::*;
    use core::task::Poll;
    use rustic_fs::{Error as FsError, PollDisk};

    /// A disk the refusals under test must never reach.
    struct Untouched;

    impl Disk for Untouched {
        fn read(&mut self, _: u64, _: &mut [u8; 512]) -> Result<(), FsError> {
            panic!("a refused request read the disk")
        }
        fn write(&mut self, _: u64, _: &[u8; 512]) -> Result<(), FsError> {
            panic!("a refused request wrote the disk")
        }
        fn flush(&mut self) -> Result<(), FsError> {
            panic!("a refused request flushed the disk")
        }
    }

    impl PollDisk for Untouched {
        fn poll_write(&mut self, _: u64, _: &[u8; 512]) -> Poll<Result<(), FsError>> {
            panic!("a refused request wrote the disk")
        }
        fn poll_flush(&mut self) -> Poll<Result<(), FsError>> {
            panic!("a refused request flushed the disk")
        }
    }

    impl PollDisk7 for Untouched {
        fn poll_read(&mut self, _: u64, _: &mut [u8; 512]) -> Poll<Result<(), FsError>> {
            panic!("a refused request read the disk")
        }
    }

    fn grant(rights: u8) -> Grant7 {
        Grant7 {
            peer: 1,
            endpoint: 2,
            context: 3,
            scope: 4,
            rights,
            subject: 2,
            expires: 0,
        }
    }

    // No installable profile holds CANCEL without INSPECT, so the rule is
    // checked here, below the grant table, on an unmounted volume.
    #[test]
    fn cancel_without_inspection_is_denied_before_any_lookup() {
        let mut volume = Volume7::EMPTY;
        let mut writes = Writes::new(&volume);
        let id = AdmissionId::new([7; 16], 9).unwrap();
        let cancel = id.packet(a::CANCEL, 3).unwrap();
        let mut grants = Grants::new();
        let mut control = |_: &mut super::super::Control7<'_>| -> u64 {
            panic!("a refused request started a publication")
        };
        let caller = Caller7 {
            slot: 0,
            peer: 1,
            context: 3,
        };
        let mut owner = Owner::new(&mut grants, caller, &mut control);
        let denied = request(
            &mut writes,
            &mut volume,
            &mut Untouched,
            0,
            grant(CANCEL_RIGHT),
            cancel,
            &mut owner,
        );
        assert_eq!(denied, Err(Error::Denied));
        // With both rights the request passes the check and reaches the
        // (unmounted) volume, which refuses it for another reason.
        let reached = request(
            &mut writes,
            &mut volume,
            &mut Untouched,
            0,
            grant(CANCEL_RIGHT | INSPECT_RIGHT),
            cancel,
            &mut owner,
        );
        assert!(matches!(reached, Err(error) if error != Error::Denied));
    }
}
