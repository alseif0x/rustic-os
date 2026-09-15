// SPDX-License-Identifier: Apache-2.0
//! Pure word translation for the persistent tasks-owner child.
//!
//! Only the shape of an owner request is decided here: every request is
//! validated against the product contract and re-encoded in the private child
//! layout. Process identity, role gating and delivery stay in the supervisor
//! binary, so this translation is exercised directly by host tests.
use rustic_sdk::abi::supervisor as s;
use rustic_tasks_contract::{MAX_BYTES, candidate, preview};

/// The highest failure cut the tasks-owner child implements, as
/// `apps/utility/src/tasks/owner.rs` maps the selector.
#[cfg(feature = "tasks-acceptance")]
const MAX_CUT: u64 = 4;

/// Translate one owner request into the child message it delivers.
///
/// The error value is the owner reply status: `1` for an invalid request. A
/// successful translation proves the shape only; the child still owns the
/// intent, its journal and every file effect.
pub fn translate(w: [u64; 8]) -> Result<[u64; 8], u64> {
    match w[0] {
        s::TASKS_OWNER_BEGIN => {
            if w[2] == 0 || w[2] > MAX_BYTES as u64 {
                return Err(1);
            }
            // The summary is canonical or the request is refused: this makes
            // `version` and `task_id` non-zero, `changed` a boolean and `count`
            // bounded. Words 4..6 of a summary are always zero and are not
            // carried on the wire.
            let summary = preview::Summary::decode([w[3], w[4], w[5], w[6], 0, 0, 0])
                .ok_or(1u64)?
                .words();
            Ok([
                s::actor::TASKS_BEGIN,
                w[2],
                summary[0],
                summary[1],
                summary[2],
                summary[3],
                0,
                0,
            ])
        }
        s::TASKS_OWNER_EDIT => {
            let e = preview::Edit::decode(w[2..].try_into().map_err(|_| 1u64)?)
                .ok_or(1u64)?
                .words();
            Ok([s::actor::TASKS_EDIT, e[0], e[1], e[2], e[3], e[4], e[5], 0])
        }
        s::TASKS_OWNER_CHUNK => {
            // Validated with the product codec instead of a second packing
            // rule: a self-contained chunk has offset 0 and total equal to its
            // length, so a successful decode proves the 32-byte bound and the
            // zero padding after `length`.
            let chunk =
                candidate::decode_owner([0, 0, w[2], w[2], w[3], w[4], w[5], w[6]]).ok_or(1u64)?;
            Ok([
                s::actor::TASKS_CHUNK,
                chunk.length,
                w[3],
                w[4],
                w[5],
                w[6],
                0,
                0,
            ])
        }
        // The key names recovery evidence the child wrote; it is opaque here.
        s::TASKS_OWNER_FORGET => Ok([s::actor::TASKS_FORGET, w[2], 0, 0, 0, 0, 0, 0]),
        // An apply under an explicit failure cut is an apply: it becomes the
        // same child action, so it keeps the same delivery window, and only the
        // cut selector is added. The whole request exists only in an acceptance
        // build; elsewhere it falls through to the invalid arm below.
        #[cfg(feature = "tasks-acceptance")]
        s::TASKS_OWNER_APPLY_CUT => {
            if w[2] > MAX_CUT {
                return Err(1);
            }
            Ok([s::actor::TASKS_APPLY, w[2], 0, 0, 0, 0, 0, 0])
        }
        _ => Err(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustic_tasks_contract::{MAX_TASKS, candidate::MAX_CHUNK};

    fn begin(total: u64, count: u64, version: u64, task_id: u64, changed: u64) -> [u64; 8] {
        [
            s::TASKS_OWNER_BEGIN,
            7,
            total,
            count,
            version,
            task_id,
            changed,
            0,
        ]
    }

    fn chunk_request(bytes: &[u8]) -> [u64; 8] {
        let words = candidate::owner_words(&candidate::chunk(bytes, 0).unwrap());
        [
            s::TASKS_OWNER_CHUNK,
            7,
            words[3],
            words[4],
            words[5],
            words[6],
            words[7],
            0,
        ]
    }

    #[test]
    fn begin_carries_the_canonical_summary_with_the_candidate_length() {
        assert_eq!(
            translate(begin(40, 2, 9, 42, 1)),
            Ok([s::actor::TASKS_BEGIN, 40, 2, 9, 42, 1, 0, 0])
        );
        // An empty document is a valid starting point for the first task.
        assert_eq!(
            translate(begin(1, 0, 1, 1, 0)),
            Ok([s::actor::TASKS_BEGIN, 1, 0, 1, 1, 0, 0, 0])
        );
    }

    #[test]
    fn begin_refuses_unbounded_lengths_and_noncanonical_summaries() {
        assert_eq!(translate(begin(0, 2, 9, 42, 1)), Err(1));
        assert_eq!(translate(begin(MAX_BYTES as u64 + 1, 2, 9, 42, 1)), Err(1));
        assert_eq!(
            translate(begin(MAX_BYTES as u64, 2, 9, 42, 1)).map(|w| w[1]),
            Ok(MAX_BYTES as u64)
        );
        // Unversioned or unidentified edits, a non-boolean flag and a count
        // outside the bounded document are all refused before any delivery.
        assert_eq!(translate(begin(40, 2, 0, 42, 1)), Err(1));
        assert_eq!(translate(begin(40, 2, 9, 0, 1)), Err(1));
        assert_eq!(translate(begin(40, 2, 9, 42, 2)), Err(1));
        assert_eq!(translate(begin(40, MAX_TASKS as u64 + 1, 9, 42, 1)), Err(1));
    }

    #[test]
    fn edit_is_forwarded_only_in_its_canonical_encoding() {
        for edit in [
            preview::Edit::add(b"Review Rust").unwrap(),
            preview::Edit::Done { id: 3 },
        ] {
            let e = edit.words();
            let request = [s::TASKS_OWNER_EDIT, 7, e[0], e[1], e[2], e[3], e[4], e[5]];
            assert_eq!(
                translate(request),
                Ok([s::actor::TASKS_EDIT, e[0], e[1], e[2], e[3], e[4], e[5], 0])
            );
            let mut malformed = request;
            malformed[7] = 1;
            assert_eq!(translate(malformed), Err(1));
        }
        // A title length with no title, and an unknown edit kind.
        assert_eq!(
            translate([s::TASKS_OWNER_EDIT, 7, 1, 1, 0, 0, 0, 0]),
            Err(1)
        );
        assert_eq!(
            translate([s::TASKS_OWNER_EDIT, 7, 3, 1, 0, 0, 0, 0]),
            Err(1)
        );
    }

    #[test]
    fn chunk_keeps_the_product_byte_order_within_its_bound() {
        let request = chunk_request(b"Hello");
        let child = translate(request).unwrap();
        assert_eq!(child[0], s::actor::TASKS_CHUNK);
        assert_eq!(child[1], 5);
        assert_eq!(&child[2..6], &request[3..7]);
        assert_eq!(child[6..], [0, 0]);
        let decoded = candidate::decode_owner([
            0, 0, child[1], child[1], child[2], child[3], child[4], child[5],
        ])
        .unwrap();
        assert_eq!(decoded.bytes(), b"Hello");
        assert_eq!(
            translate(chunk_request(&[b'x'; MAX_CHUNK])).unwrap()[1],
            MAX_CHUNK as u64
        );
    }

    #[test]
    fn chunk_refuses_empty_oversized_and_unpadded_payloads() {
        let mut request = chunk_request(&[b'x'; MAX_CHUNK]);
        request[2] = 0;
        assert_eq!(translate(request), Err(1));
        request[2] = MAX_CHUNK as u64 + 1;
        assert_eq!(translate(request), Err(1));
        // Bytes after the declared length must be zero.
        let mut short = chunk_request(b"A");
        short[4] = 1;
        assert_eq!(translate(short), Err(1));
    }

    #[cfg(feature = "tasks-acceptance")]
    #[test]
    fn an_apply_cut_is_an_apply_carrying_only_an_implemented_selector() {
        for cut in 0..=MAX_CUT {
            assert_eq!(
                translate([s::TASKS_OWNER_APPLY_CUT, 7, cut, 0, 0, 0, 0, 0]),
                Ok([s::actor::TASKS_APPLY, cut, 0, 0, 0, 0, 0, 0])
            );
        }
        // A selector no build implements is refused before delivery, so the
        // child never has to answer for a cut it does not have.
        for cut in [MAX_CUT + 1, u64::MAX] {
            assert_eq!(
                translate([s::TASKS_OWNER_APPLY_CUT, 7, cut, 0, 0, 0, 0, 0]),
                Err(1)
            );
        }
    }

    #[cfg(not(feature = "tasks-acceptance"))]
    #[test]
    fn an_apply_cut_does_not_exist_outside_an_acceptance_build() {
        for cut in 0..=5 {
            assert_eq!(
                translate([s::TASKS_OWNER_APPLY_CUT, 7, cut, 0, 0, 0, 0, 0]),
                Err(1)
            );
        }
    }

    #[test]
    fn forget_passes_its_opaque_key_and_unknown_requests_are_invalid() {
        assert_eq!(
            translate([s::TASKS_OWNER_FORGET, 7, 91, 0, 0, 0, 0, 0]),
            Ok([s::actor::TASKS_FORGET, 91, 0, 0, 0, 0, 0, 0])
        );
        assert_eq!(translate([s::TASKS_LIST, 7, 0, 0, 0, 0, 0, 0]), Err(1));
        assert_eq!(translate([0; 8]), Err(1));
    }
}
