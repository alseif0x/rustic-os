// SPDX-License-Identifier: Apache-2.0
//! Which deterministic actor action exists, which one the owner may ask for
//! directly, and which role may answer it.
//!
//! These are pure decisions about numbers: no process identity, no child state
//! and no delivery. The supervisor binary keeps the table of children, the
//! pending exchange and the deadline; it asks this module only whether a number
//! is admissible at all, so the admission rules can be exercised on the host.
//!
//! Three sets exist and they are deliberately different:
//!
//! - [`tasks_action`] is the persistent tasks-owner protocol, the contiguous
//!   block `TASKS_BEGIN ..= TASKS_HEAP_STRESS`.
//! - [`actor_allowed`] is what the owner's `ACT` verb may name. It excludes the
//!   collection steps, which only the owner tasks requests produce through
//!   [`super::tasks_owner::translate`], and the actions the supervisor builds
//!   itself, such as `MOVED` and `ADMISSION`.
//! - [`role_admits`] pairs the two vocabularies: a tasks-owner child answers the
//!   tasks protocol and nothing else, and no other role answers any of it.
use rustic_sdk::abi::supervisor as s;

/// Whether an actor action belongs to the persistent tasks-owner protocol.
pub fn tasks_action(action: u64) -> bool {
    matches!(
        action,
        s::actor::TASKS_BEGIN
            | s::actor::TASKS_EDIT
            | s::actor::TASKS_CHUNK
            | s::actor::TASKS_APPLY
            | s::actor::TASKS_STATUS
            | s::actor::TASKS_RECOVER
            | s::actor::TASKS_FORGET
            | s::actor::TASKS_HEAP_STRESS
    )
}

/// Whether the owner may name this action in an `ACT` request.
///
/// The collection steps of the tasks protocol are absent on purpose: their
/// words carry a candidate and are validated by the product contract before
/// delivery, so they exist only as owner tasks requests.
pub fn actor_allowed(action: u64) -> bool {
    matches!(
        action,
        s::actor::READ
            | s::actor::STAGE
            | s::actor::COMMIT
            | s::actor::FLOOD
            | s::actor::DRAIN
            | s::actor::STALE
            | s::actor::API_READ
            | s::actor::READ_OPEN
            | s::actor::READ_NEXT
            | s::actor::FILL
            | s::actor::OPERATION_GET
            | s::actor::CAPABILITIES
            | s::actor::PROFILE_GET
            | s::actor::PROFILE_CANCEL
            | s::actor::SELECT_GET
            | s::actor::SELECT_CANCEL
            | s::actor::MISSION_PREPARE
            | s::actor::MISSION_VERIFY
            | s::actor::MISSION_SCHEDULE
            | s::actor::MISSION_INSPECT
            | s::actor::MISSION_CANCEL
            | s::actor::TASKS_APPLY
            | s::actor::TASKS_STATUS
            | s::actor::TASKS_RECOVER
            | s::actor::TASKS_HEAP_STRESS
    )
}

/// Whether a child holding `role` may be asked `action` at all.
///
/// One rule decides both directions: the tasks protocol is exactly the
/// vocabulary of [`s::TASKS_OWNER`], so that role refuses every read, mission
/// and admission action, and every other role refuses every tasks action.
pub fn role_admits(role: u64, action: u64) -> bool {
    tasks_action(action) == (role == s::TASKS_OWNER)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every number the actor vocabulary could name, plus room above it, so a
    /// future constant is classified deliberately instead of by accident.
    const RANGE: core::ops::RangeInclusive<u64> = 0..=64;

    /// The roles a child may hold. `TASKS` is the read-only tasks application,
    /// which is not the owner-stepped client.
    const ROLES: [u64; 4] = [s::SESSION, s::HELPER, s::TASKS, s::TASKS_OWNER];

    fn act_verbs() -> [u64; 25] {
        [
            s::actor::READ,
            s::actor::STAGE,
            s::actor::COMMIT,
            s::actor::FLOOD,
            s::actor::DRAIN,
            s::actor::STALE,
            s::actor::API_READ,
            s::actor::READ_OPEN,
            s::actor::READ_NEXT,
            s::actor::FILL,
            s::actor::OPERATION_GET,
            s::actor::CAPABILITIES,
            s::actor::PROFILE_GET,
            s::actor::PROFILE_CANCEL,
            s::actor::SELECT_GET,
            s::actor::SELECT_CANCEL,
            s::actor::MISSION_PREPARE,
            s::actor::MISSION_VERIFY,
            s::actor::MISSION_SCHEDULE,
            s::actor::MISSION_INSPECT,
            s::actor::MISSION_CANCEL,
            s::actor::TASKS_APPLY,
            s::actor::TASKS_STATUS,
            s::actor::TASKS_RECOVER,
            s::actor::TASKS_HEAP_STRESS,
        ]
    }

    #[test]
    fn the_tasks_protocol_is_exactly_the_contiguous_block_of_its_opcodes() {
        assert_eq!(s::actor::TASKS_BEGIN, 24);
        assert_eq!(s::actor::TASKS_HEAP_STRESS, 31);
        for action in RANGE {
            assert_eq!(
                tasks_action(action),
                (s::actor::TASKS_BEGIN..=s::actor::TASKS_HEAP_STRESS).contains(&action),
                "action {action} changed protocol"
            );
        }
        // The neighbours are ordinary actions and must stay outside.
        assert!(!tasks_action(s::actor::MISSION_CANCEL));
        assert!(!tasks_action(s::actor::TASKS_HEAP_STRESS + 1));
    }

    #[test]
    fn act_names_the_documented_verbs_and_no_collection_step() {
        let verbs = act_verbs();
        for action in RANGE {
            assert_eq!(
                actor_allowed(action),
                verbs.contains(&action),
                "act admission changed for action {action}"
            );
        }
        // A candidate never arrives through `act`: these four are produced only
        // by the owner tasks requests, which validate their words first.
        for step in [
            s::actor::TASKS_BEGIN,
            s::actor::TASKS_EDIT,
            s::actor::TASKS_CHUNK,
            s::actor::TASKS_FORGET,
        ] {
            assert!(!actor_allowed(step), "collection step {step} reachable");
            assert!(tasks_action(step));
        }
        // The supervisor builds these itself; the owner cannot name them.
        for internal in [s::actor::MOVED, s::actor::ADMISSION] {
            assert!(!actor_allowed(internal));
        }
        // An unknown number is refused, including the one after the last verb.
        assert!(!actor_allowed(0));
        assert!(!actor_allowed(s::actor::TASKS_HEAP_STRESS + 1));
    }

    #[test]
    fn the_stress_step_is_an_act_verb_that_only_a_tasks_owner_answers() {
        assert!(actor_allowed(s::actor::TASKS_HEAP_STRESS));
        assert!(tasks_action(s::actor::TASKS_HEAP_STRESS));
        for role in ROLES {
            assert_eq!(
                role_admits(role, s::actor::TASKS_HEAP_STRESS),
                role == s::TASKS_OWNER
            );
        }
    }

    #[test]
    fn a_role_answers_one_vocabulary_and_never_the_other() {
        for action in RANGE {
            for role in ROLES {
                let admitted = role_admits(role, action);
                if role == s::TASKS_OWNER {
                    assert_eq!(admitted, tasks_action(action), "owner took {action}");
                } else {
                    assert_eq!(admitted, !tasks_action(action), "role {role} took {action}");
                }
            }
        }
        // The read-only tasks application shares the name, not the protocol.
        assert!(!role_admits(s::TASKS, s::actor::TASKS_STATUS));
        assert!(role_admits(s::TASKS, s::actor::READ_OPEN));
        assert!(!role_admits(s::TASKS_OWNER, s::actor::READ_OPEN));
    }

    #[test]
    fn every_translated_owner_request_is_a_tasks_action_for_the_owner_role() {
        use crate::tasks_owner::translate;
        use rustic_tasks_contract::{candidate, preview};
        let edit = preview::Edit::Done { id: 3 }.words();
        let chunk = candidate::owner_words(&candidate::chunk(b"hi", 0).unwrap());
        // One representative of each owner request, in its canonical shape.
        let requests = [
            [s::TASKS_OWNER_BEGIN, 3, 7, 2, 9, 42, 1, 0],
            [
                s::TASKS_OWNER_EDIT,
                3,
                edit[0],
                edit[1],
                edit[2],
                edit[3],
                edit[4],
                edit[5],
            ],
            [
                s::TASKS_OWNER_CHUNK,
                3,
                chunk[3],
                chunk[4],
                chunk[5],
                chunk[6],
                chunk[7],
                0,
            ],
            [s::TASKS_OWNER_FORGET, 3, 11, 0, 0, 0, 0, 0],
        ];
        for request in requests {
            let child = translate(request).expect("canonical owner request");
            assert!(
                tasks_action(child[0]),
                "request {request:?} left the protocol"
            );
            assert!(role_admits(s::TASKS_OWNER, child[0]));
            for role in ROLES.iter().filter(|role| **role != s::TASKS_OWNER) {
                assert!(!role_admits(*role, child[0]));
            }
            // None of these four may be named directly by the owner's `act`.
            assert!(!actor_allowed(child[0]));
        }
    }
}
