// SPDX-License-Identifier: Apache-2.0
//! Administrative grant sequence of one dormant child, without transport.
//!
//! The supervisor binary owns the endpoints, the process and the exchanges; this
//! module owns only the order of the private administrative operations and the
//! words they carry, so every transition is exercised directly by host tests.
//!
//! The invariant it enforces is that an accepted root is never left behind: once
//! the file service has installed a grant for a child that is still dormant, the
//! sequence can end only by activating that child or by withdrawing the root.
use rustic_sdk::abi::{
    files::{GRANT, GRANT_SECOND_SCOPE, REVOKE},
    supervisor as s,
};

/// Private derivation of one helper level from a live root. It has no shared ABI
/// constant because this sequence is the only issuer.
const DERIVE: u64 = 37;

/// Client slots 0 and 1 of the file service hold the shell and the supervisor's
/// own owner binding, so child slot `n` is administered as client slot `n + 2`.
const RESERVED: usize = 2;

/// The recovery identity of the shell's owner client and of the supervisor. Roles
/// that act on the owner's behalf share it; roles that keep their own durable
/// operations must not.
const OWNER_SUBJECT: u64 = 1;

/// Everything the service is told about one child's authority.
///
/// It is rebuilt from the draft on every step instead of being stored twice: the
/// supervisor owns these facts, this module only formats and orders them.
#[derive(Clone, Copy)]
pub struct Request {
    pub slot: usize,
    pub role: u64,
    /// Process that will hold the grant. It stays dormant until activation.
    pub peer: u64,
    /// Service-side endpoint token the child will answer on.
    pub endpoint: u64,
    pub scope: u32,
    /// Second object. Granted only to [`s::TASKS_OWNER`]; merely forwarded to the
    /// child for every other role.
    pub other: u32,
    pub rights: u8,
    pub expires: u64,
    /// Live root a helper is derived from; zero for a standalone grant.
    pub parent: u32,
}

impl Request {
    fn client(&self) -> u64 {
        (self.slot + RESERVED) as u64
    }

    /// Recovery identity installed with the grant; zero means no durable-operation
    /// authority at all.
    ///
    /// A retry key is only ever compared inside one subject, so two clients that
    /// retain their own intents must not share one. The tasks owner is therefore
    /// identified by the journal object it was granted: that identity is the
    /// record it recovers from, it is stable across relaunches on the same
    /// journal, and it can never be the owner subject because the supervisor
    /// refuses the role unless the journal object is above it.
    pub fn subject(&self) -> u64 {
        match self.role {
            s::PRIVATE_ADMISSION_SESSION => self.peer,
            s::TASKS_OWNER => u64::from(self.other),
            s::LOST_REPLY | s::LOST_OPERATION | s::LOST_ADMISSION | s::ADMISSION_SESSION => {
                OWNER_SUBJECT
            }
            _ => 0,
        }
    }

    /// Only the two-scope role receives authority over its second object.
    fn extends(&self) -> bool {
        self.role == s::TASKS_OWNER
    }
}

/// Where the sequence stands with respect to the file service.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Nothing is installed yet: the root grant or the helper derivation is owed.
    Install,
    /// The root is installed and its second object scope is being attached.
    Extend,
    /// The root is installed and the child may now be started with it.
    Ready,
    /// The root is installed and the child will never run: it is being withdrawn.
    Withdraw,
    /// Nothing is owed to the service and no root is held for a dormant child.
    Done,
}

/// What the caller must do after one transition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    /// One more administrative exchange is required before the job can end.
    Again,
    /// The installed root may be handed to the child under this generation.
    Ready(u32),
    /// The job ends with this owner error and no root is left for a dead child.
    Failed(u64),
}

/// Phase and generation of one child's administrative sequence.
pub struct Sequence {
    phase: Phase,
    generation: u32,
    failure: u64,
    leaked: bool,
}

impl Default for Sequence {
    fn default() -> Self {
        Self::new()
    }
}

impl Sequence {
    pub fn new() -> Self {
        Self {
            phase: Phase::Install,
            generation: 0,
            failure: 0,
            leaked: false,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// A root is installed in the service and no running child holds it.
    pub fn outstanding(&self) -> bool {
        self.generation != 0 && self.phase != Phase::Done
    }

    /// The withdrawal was refused or could not be completed, so a root bound to a
    /// child that never ran may still be installed. The caller must degrade.
    pub fn leaked(&self) -> bool {
        self.leaked
    }

    /// Words of the exchange this phase owes the service.
    ///
    /// [`Phase::Done`] owes nothing; its zero words are never sent because the job
    /// has already ended.
    pub fn words(&self, r: &Request) -> [u64; 8] {
        match self.phase {
            Phase::Extend => [
                GRANT_SECOND_SCOPE as u64,
                r.client(),
                u64::from(self.generation),
                u64::from(r.other),
                0,
                0,
                0,
                0,
            ],
            Phase::Withdraw => self.withdrawal(r),
            Phase::Install if r.parent != 0 => [
                DERIVE,
                r.client(),
                u64::from(r.parent),
                r.peer,
                r.endpoint,
                u64::from(r.scope),
                u64::from(r.rights),
                r.expires,
            ],
            Phase::Install => [
                GRANT as u64,
                r.client(),
                r.peer,
                r.endpoint,
                u64::from(r.scope),
                u64::from(r.rights),
                r.expires,
                r.subject(),
            ],
            Phase::Ready | Phase::Done => [0; 8],
        }
    }

    /// Words that withdraw the installed root, in any phase.
    ///
    /// The whole grant goes, including a second scope that may or may not have
    /// been attached: the slot is the unit of revocation.
    pub fn withdrawal(&self, r: &Request) -> [u64; 8] {
        [REVOKE as u64, r.client(), 0, 0, 0, 0, 0, 0]
    }

    /// Interpret one decoded administrative reply.
    pub fn reply(&mut self, r: &Request, w: [u64; 8]) -> Step {
        match self.phase {
            Phase::Withdraw => {
                // Any answer concludes the job with the original failure. A refusal
                // leaves the root's fate unknown, which is a degraded supervisor.
                self.leaked = w[0] != 0;
                self.phase = Phase::Done;
                Step::Failed(self.failure)
            }
            Phase::Ready | Phase::Done => self.fail(4),
            Phase::Install | Phase::Extend => {
                if w[0] != 0 {
                    return self.fail(2);
                }
                // The service accepted, so a root is installed even when the rest
                // of the reply is unusable. Its generation may be unknown, but the
                // withdrawal addresses the slot, so the root is withdrawn anyway.
                let generation = match u32::try_from(w[1]) {
                    Ok(generation) if generation != 0 && w[2..].iter().all(|v| *v == 0) => {
                        generation
                    }
                    _ => return self.withdraw(4),
                };
                if self.phase == Phase::Install {
                    self.generation = generation;
                    if r.extends() {
                        // The child stays dormant until its journal record is
                        // attached to the root that was just installed.
                        self.phase = Phase::Extend;
                        return Step::Again;
                    }
                } else if generation != self.generation {
                    // The service echoes the unchanged generation; anything else
                    // means the grant this sequence owns is not the one extended.
                    return self.fail(4);
                }
                self.phase = Phase::Ready;
                Step::Ready(generation)
            }
        }
    }

    /// The exchange, a precondition, the activation or the job deadline failed.
    ///
    /// A sequence that installed nothing simply ends. Once the service accepted the
    /// root, the job cannot end before that root is withdrawn.
    pub fn fail(&mut self, error: u64) -> Step {
        match self.phase {
            Phase::Withdraw => Step::Again,
            Phase::Done => Step::Failed(error),
            _ if self.generation == 0 => {
                self.phase = Phase::Done;
                Step::Failed(error)
            }
            _ => self.withdraw(error),
        }
    }

    /// An installed root must go before the job may report `error`.
    fn withdraw(&mut self, error: u64) -> Step {
        self.failure = error;
        self.phase = Phase::Withdraw;
        Step::Again
    }

    /// The child is running and owns the grant; nothing is owed any more.
    pub fn activated(&mut self) {
        self.phase = Phase::Done;
    }

    /// The administrative channel cannot carry another exchange. Nothing more can
    /// be proven about an installed root, so the job ends and the caller degrades.
    pub fn abandon(&mut self, error: u64) -> u64 {
        self.leaked = self.outstanding();
        self.phase = Phase::Done;
        if self.failure != 0 {
            self.failure
        } else {
            error
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENERATION: u32 = 9;

    fn request(role: u64) -> Request {
        Request {
            slot: 1,
            role,
            peer: 42,
            endpoint: 7,
            scope: 5,
            other: 6,
            rights: 7,
            expires: 100,
            parent: 0,
        }
    }

    fn accepted(generation: u32) -> [u64; 8] {
        [0, u64::from(generation), 0, 0, 0, 0, 0, 0]
    }

    /// A revocation reply: rights before, fenced transfers, root state, sequence.
    fn revoked() -> [u64; 8] {
        [0, 7, 1, 0, 12, 0, 0, 0]
    }

    #[test]
    fn the_owner_root_and_its_second_scope_are_installed_in_order() {
        let r = request(s::TASKS_OWNER);
        let mut sequence = Sequence::new();
        assert_eq!(sequence.words(&r), [32, 3, 42, 7, 5, 7, 100, 6]);
        assert_eq!(sequence.reply(&r, accepted(GENERATION)), Step::Again);
        assert_eq!(sequence.phase(), Phase::Extend);
        assert_eq!(sequence.words(&r), [38, 3, 9, 6, 0, 0, 0, 0]);
        assert_eq!(
            sequence.reply(&r, accepted(GENERATION)),
            Step::Ready(GENERATION)
        );
        assert!(sequence.outstanding());
        sequence.activated();
        assert_eq!(sequence.phase(), Phase::Done);
        assert!(!sequence.outstanding() && !sequence.leaked());
    }

    #[test]
    fn the_tasks_owner_records_under_its_journal_and_never_the_owner_subject() {
        // Two clients that retain their own intents must not share a subject.
        assert_eq!(request(s::TASKS_OWNER).subject(), 6);
        assert_eq!(request(s::LOST_REPLY).subject(), OWNER_SUBJECT);
        assert_eq!(request(s::ADMISSION_SESSION).subject(), OWNER_SUBJECT);
        assert_eq!(request(s::PRIVATE_ADMISSION_SESSION).subject(), 42);
        assert_eq!(request(s::SESSION).subject(), 0);
        assert_eq!(request(s::TASKS).subject(), 0);
        // The subject travels in the grant itself, not in the extension.
        let r = request(s::TASKS_OWNER);
        assert_eq!(Sequence::new().words(&r)[7], u64::from(r.other));
    }

    #[test]
    fn a_refused_second_scope_withdraws_the_installed_root() {
        for refusal in [[2, 0, 0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0, 0, 0]] {
            let r = request(s::TASKS_OWNER);
            let mut sequence = Sequence::new();
            assert_eq!(sequence.reply(&r, accepted(GENERATION)), Step::Again);
            assert_eq!(sequence.reply(&r, refusal), Step::Again);
            assert_eq!(sequence.phase(), Phase::Withdraw);
            assert_eq!(sequence.words(&r), [33, 3, 0, 0, 0, 0, 0, 0]);
            let error = if refusal[0] == 0 { 4 } else { 2 };
            assert_eq!(sequence.reply(&r, revoked()), Step::Failed(error));
            assert!(!sequence.outstanding() && !sequence.leaked());
        }
    }

    #[test]
    fn an_extension_of_another_generation_is_refused_and_withdrawn() {
        let r = request(s::TASKS_OWNER);
        let mut sequence = Sequence::new();
        assert_eq!(sequence.reply(&r, accepted(GENERATION)), Step::Again);
        assert_eq!(sequence.reply(&r, accepted(GENERATION + 1)), Step::Again);
        assert_eq!(sequence.words(&r), [33, 3, 0, 0, 0, 0, 0, 0]);
        assert_eq!(sequence.reply(&r, revoked()), Step::Failed(4));
    }

    #[test]
    fn expiry_while_extending_still_withdraws_the_root() {
        let r = request(s::TASKS_OWNER);
        let mut sequence = Sequence::new();
        assert_eq!(sequence.reply(&r, accepted(GENERATION)), Step::Again);
        assert_eq!(sequence.fail(4), Step::Again);
        assert_eq!(sequence.words(&r), [33, 3, 0, 0, 0, 0, 0, 0]);
        // A second deadline cannot restart the withdrawal, and a refused
        // revocation is reported as a root of unknown fate.
        assert_eq!(sequence.fail(4), Step::Again);
        assert_eq!(
            sequence.reply(&r, [4, 0, 0, 0, 0, 0, 0, 0]),
            Step::Failed(4)
        );
        assert!(sequence.leaked());
    }

    #[test]
    fn a_failed_activation_withdraws_the_root_it_could_not_hand_over() {
        let r = request(s::SESSION);
        let mut sequence = Sequence::new();
        assert_eq!(
            sequence.reply(&r, accepted(GENERATION)),
            Step::Ready(GENERATION)
        );
        assert_eq!(sequence.fail(4), Step::Again);
        assert_eq!(sequence.words(&r), [33, 3, 0, 0, 0, 0, 0, 0]);
        assert_eq!(sequence.reply(&r, revoked()), Step::Failed(4));
        // An unusable channel ends the job and reports the root as unknown.
        let mut sequence = Sequence::new();
        assert_eq!(
            sequence.reply(&r, accepted(GENERATION)),
            Step::Ready(GENERATION)
        );
        assert_eq!(sequence.abandon(4), 4);
        assert!(sequence.leaked());
    }

    #[test]
    fn roles_without_a_second_scope_never_extend_or_withdraw_early() {
        for role in [s::SESSION, s::TASKS, s::READ, s::ADMISSION_SESSION] {
            let r = request(role);
            let mut sequence = Sequence::new();
            assert_eq!(sequence.words(&r)[0], 32);
            // Nothing is installed yet, so a refusal ends the job at once.
            assert_eq!(sequence.fail(2), Step::Failed(2));
            assert_eq!(sequence.phase(), Phase::Done);
            assert!(!sequence.outstanding() && !sequence.leaked());
            let mut sequence = Sequence::new();
            assert_eq!(
                sequence.reply(&r, accepted(GENERATION)),
                Step::Ready(GENERATION)
            );
            sequence.activated();
            assert_eq!(sequence.words(&r), [0; 8]);
        }
    }

    #[test]
    fn a_helper_is_derived_from_its_parent_root_without_a_subject() {
        let r = Request {
            parent: 3,
            ..request(s::HELPER)
        };
        let mut sequence = Sequence::new();
        assert_eq!(sequence.words(&r), [37, 3, 3, 42, 7, 5, 7, 100]);
        assert_eq!(
            sequence.reply(&r, accepted(GENERATION)),
            Step::Ready(GENERATION)
        );
        // A helper whose parent session died must not keep the derived root.
        assert_eq!(sequence.fail(2), Step::Again);
        assert_eq!(sequence.words(&r), [33, 3, 0, 0, 0, 0, 0, 0]);
        assert_eq!(sequence.reply(&r, revoked()), Step::Failed(2));
    }

    #[test]
    fn an_accepted_but_unusable_reply_still_withdraws_the_installed_root() {
        let r = request(s::SESSION);
        for reply in [
            [0, 0, 0, 0, 0, 0, 0, 0],
            [0, u64::from(u32::MAX) + 1, 0, 0, 0, 0, 0, 0],
            [0, 9, 1, 0, 0, 0, 0, 0],
        ] {
            let mut sequence = Sequence::new();
            // The service said yes, so something is installed: the sequence may
            // not end before the slot is revoked, generation known or not.
            assert_eq!(sequence.reply(&r, reply), Step::Again);
            assert_eq!(sequence.phase(), Phase::Withdraw);
            assert_eq!(sequence.words(&r), [33, 3, 0, 0, 0, 0, 0, 0]);
            assert_eq!(sequence.reply(&r, revoked()), Step::Failed(4));
            assert_eq!(sequence.phase(), Phase::Done);
            assert!(!sequence.leaked());
        }
    }

    #[test]
    fn a_reply_after_activation_changes_nothing() {
        let r = request(s::SESSION);
        // A reply that arrives once the sequence is over changes nothing.
        let mut sequence = Sequence::new();
        assert_eq!(
            sequence.reply(&r, accepted(GENERATION)),
            Step::Ready(GENERATION)
        );
        sequence.activated();
        assert_eq!(sequence.reply(&r, accepted(GENERATION)), Step::Failed(4));
        assert!(!sequence.outstanding());
    }
}
