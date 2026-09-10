// SPDX-License-Identifier: Apache-2.0
//! Retained bounded takeover facts, independent of transport and kernel mechanisms.
pub const REQUESTED: u64 = 1;
pub const UNCONFIRMED: u64 = 2;
pub const FENCED: u64 = 3;
pub const UNKNOWN: u64 = 0;
pub const SETTLED: u64 = 1;
pub const RECOVERY_REQUIRED: u64 = 2;
#[derive(Clone, Copy)]
pub struct Record {
    pub server: u64,
    pub root: u32,
    pub members: [u64; 2],
    pub mask: u64,
    deadline: u64,
    fenced: bool,
    unavailable: bool,
    discarded: u64,
    effects: u64,
    sequence: u64,
}
impl Record {
    pub fn new(server: u64, root: u32, members: [u64; 2], mask: u64, now: u64) -> Self {
        Self {
            server,
            root,
            members,
            mask,
            deadline: now.saturating_add(200),
            fenced: false,
            unavailable: false,
            discarded: u64::MAX,
            effects: UNKNOWN,
            sequence: 0,
        }
    }
    pub fn pending(&self) -> bool {
        !self.fenced
    }
    pub fn contains(&self, pid: u64) -> bool {
        pid != 0 && self.members.contains(&pid)
    }
    pub fn missing(&mut self) {
        self.unavailable = true;
    }
    pub fn confirm(&mut self, server: u64, w: [u64; 8]) -> bool {
        if self.fenced {
            return false;
        }
        if server != self.server
            || w[0] != 0
            || w[1] & !self.mask != 0
            || w[2] > 2
            || w[3] > 1
            || w[5..].iter().any(|v| *v != 0)
        {
            self.missing();
            return false;
        }
        self.fenced = true;
        self.discarded = w[2];
        self.sequence = w[4];
        self.effects = if w[3] == 0 {
            SETTLED
        } else {
            RECOVERY_REQUIRED
        };
        true
    }
    /// Old processes and service have ended. This proves fencing, not an I/O outcome.
    pub fn service_ended(&mut self, server: u64) {
        if self.server == server && !self.fenced {
            self.fenced = true;
            self.effects = RECOVERY_REQUIRED;
        }
    }
    pub fn words(&self, now: u64) -> [u64; 8] {
        let phase = if self.fenced {
            FENCED
        } else if self.unavailable || now >= self.deadline {
            UNCONFIRMED
        } else {
            REQUESTED
        };
        [
            0,
            phase,
            self.members.iter().filter(|p| **p != 0).count() as u64,
            self.discarded,
            self.effects,
            self.sequence,
            self.root as u64,
            self.server,
        ]
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deadline_and_missing_ack_never_claim_fenced_or_settled() {
        let mut r = Record::new(2, 3, [4, 5], 12, 10);
        assert_eq!(r.words(209)[1], REQUESTED);
        assert_eq!(r.words(210)[1], UNCONFIRMED);
        assert_eq!(r.words(210)[4], UNKNOWN);
        r.missing();
        assert_eq!(r.words(10)[1], UNCONFIRMED);
        assert!(r.pending());
    }
    #[test]
    fn late_ack_is_bound_to_service_and_reports_uncertain_effects_separately() {
        let mut r = Record::new(2, 3, [4, 5], 12, 0);
        assert!(!r.confirm(7, [0, 12, 1, 0, 9, 0, 0, 0]));
        assert!(!r.confirm(2, [0, 15, 1, 0, 9, 0, 0, 0]));
        assert!(r.confirm(2, [0, 12, 1, 1, 9, 0, 0, 0]));
        assert_eq!(
            r.words(10000),
            [0, FENCED, 2, 1, RECOVERY_REQUIRED, 9, 3, 2]
        );
    }
    #[test]
    fn service_retirement_preserves_unknown_effects_and_old_identity() {
        let mut r = Record::new(2, 3, [4, 5], 12, 0);
        r.service_ended(7);
        assert!(r.pending());
        r.service_ended(2);
        assert!(!r.confirm(2, [0, 12, 0, 0, 9, 0, 0, 0]));
        assert_eq!(
            r.words(1),
            [0, FENCED, 2, u64::MAX, RECOVERY_REQUIRED, 0, 3, 2]
        );
        assert!(r.contains(4));
        assert!(r.contains(5));
        assert!(!r.contains(0));
    }
}
