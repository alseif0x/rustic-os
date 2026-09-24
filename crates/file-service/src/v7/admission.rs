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
//! Each publication is driven to settlement inside its request; no other
//! request is served meanwhile. The volume enforces versions, retry scopes,
//! the retained-record budget and publication barriers.
mod records;

use super::settle::settle;
use super::write::Writes;
use super::{Grant7, scope};
use crate::{disk::Synchronous, reply};
use rustic_abi::files::{
    admission::{self as a, AdmissionId},
    operation,
    workspace::{Lookup, Replacement},
    *,
};
use rustic_fs::{Disk, PreventionReason, Stage7Kind, Volume7, format7::RecordState};

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

pub(super) fn request(
    writes: &mut Writes,
    volume: &mut Volume7,
    disk: &mut impl Disk,
    slot: usize,
    grant: Grant7,
    p: Packet,
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
            let result = transfer.finish_admission(volume, disk);
            let record = published(writes, volume, result)?;
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
            let mut disk = Synchronous(disk);
            let result = volume
                .prepare_execute(&mut disk, records::identity(&record), record.previous)
                .map_err(reply::error)
                .and_then(settle);
            let committed = published(writes, volume, result)?;
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
            let mut disk = Synchronous(disk);
            let result = volume
                .prepare_cancellation(
                    &mut disk,
                    records::identity(&record),
                    record.previous,
                    PreventionReason::Requested,
                )
                .map_err(reply::error)
                .and_then(settle);
            let cancelled = published(writes, volume, result)?;
            status_reply(volume, &cancelled, &p).map_err(|_| Error::Uncertain)
        }
        _ => Err(Error::Unsupported),
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

fn status_reply(
    volume: &Volume7,
    record: &rustic_fs::format7::Record7,
    p: &Packet,
) -> Result<Packet, Error> {
    let lineage = volume.header().map_err(reply::error)?.lineage;
    records::status(lineage, record)?.packet(p.op, p.context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustic_fs::Error as FsError;

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
        let denied = request(
            &mut writes,
            &mut volume,
            &mut Untouched,
            0,
            grant(CANCEL_RIGHT),
            cancel,
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
        );
        assert!(matches!(reached, Err(error) if error != Error::Denied));
    }
}
