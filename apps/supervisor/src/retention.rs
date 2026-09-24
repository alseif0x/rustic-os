// SPDX-License-Identifier: Apache-2.0
//! The owner's V7 retention maintenance job, without transport: the
//! administrative words it sends and how the service's reply becomes the job
//! result.
//!
//! The supervisor binary owns the job (preconditions, the single exchange and
//! the deadline) for [`MAINTAIN_V7`](rustic_sdk::abi::supervisor::MAINTAIN_V7);
//! this module owns the words and the checks on the reply, so they are
//! exercised by host tests. The supervisor never decides when outcomes are
//! resolved beyond the owner's request: the file service refuses with `Busy`
//! while anything it can see is still open.
use rustic_sdk::abi::{
    files::{MAINTAIN_RETENTION, workspace::RETAINED_RECORDS},
    supervisor::maintenance,
};

/// Administrative words of the maintenance request.
pub fn request() -> [u64; 8] {
    [u64::from(MAINTAIN_RETENTION), 0, 0, 0, 0, 0, 0, 0]
}

/// The job result for a service reply: `[0, file_status, epoch, reclaimed]`,
/// or `None` for a malformed reply, which fails the job. A refusal carries
/// only its status; a success must advance the epoch by exactly one and name
/// a bounded record count and a sector count that fits the result word.
pub fn result(reply: [u64; 8]) -> Option<[u64; 8]> {
    if reply[5..].iter().any(|word| *word != 0) {
        return None;
    }
    if reply[0] != 0 {
        return (reply[0] <= u64::from(u8::MAX) && reply[1..].iter().all(|word| *word == 0))
            .then_some([0, reply[0], 0, 0, 0, 0, 0, 0]);
    }
    let [_, previous, epoch, records, sectors, ..] = reply;
    if previous == 0
        || previous.checked_add(1) != Some(epoch)
        || records > u64::from(RETAINED_RECORDS)
    {
        return None;
    }
    let sectors = u32::try_from(sectors).ok()?;
    Some([
        0,
        0,
        epoch,
        maintenance::reclaimed(records as u32, sectors),
        0,
        0,
        0,
        0,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_is_the_bare_administrative_maintenance_opcode() {
        assert_eq!(request(), [44, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_success_reports_the_new_epoch_and_what_was_reclaimed() {
        let done = result([0, 3, 4, 8, 120, 0, 0, 0]).unwrap();
        assert_eq!(done[..3], [0, 0, 4]);
        assert_eq!(maintenance::split(done[3]), (8, 120));
    }

    #[test]
    fn a_refusal_is_reported_as_the_file_status_with_nothing_else() {
        let busy = rustic_sdk::abi::files::Error::Busy as u64;
        assert_eq!(
            result([busy, 0, 0, 0, 0, 0, 0, 0]),
            Some([0, busy, 0, 0, 0, 0, 0, 0])
        );
        assert_eq!(result([busy, 3, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(result([256, 0, 0, 0, 0, 0, 0, 0]), None);
    }

    #[test]
    fn an_epoch_that_does_not_advance_by_one_or_an_impossible_count_is_malformed() {
        for reply in [
            [0, 3, 3, 1, 0, 0, 0, 0],
            [0, 3, 5, 1, 0, 0, 0, 0],
            [0, 0, 1, 1, 0, 0, 0, 0],
            [0, u64::MAX, 0, 1, 0, 0, 0, 0],
            [0, 3, 4, 9, 0, 0, 0, 0],
            [0, 3, 4, 1, 1 << 32, 0, 0, 0],
            [0, 3, 4, 1, 0, 1, 0, 0],
        ] {
            assert_eq!(result(reply), None, "{reply:?}");
        }
    }
}
