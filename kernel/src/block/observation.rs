// SPDX-License-Identifier: Apache-2.0
//! Deterministic diagnostic for delayed completion observation after real submission.
#[derive(Default)]
pub struct Hold {
    armed: Option<(u64, u64, u64)>,
    held: Option<(u64, u64, u64)>,
}
impl Hold {
    pub fn arm(&mut self, owner: u64, skip: u64, ticks: u64) -> bool {
        if owner == 0 || skip > 16 || !(1..=500).contains(&ticks) || self.held.is_some() {
            return false;
        }
        self.armed = Some((owner, skip, ticks));
        true
    }
    pub fn submitted(&mut self, owner: u64, id: u64, mutation: bool, now: u64) -> bool {
        let Some((target, skip, ticks)) = self.armed else {
            return false;
        };
        if target != owner || !mutation {
            return false;
        }
        if skip != 0 {
            self.armed = Some((target, skip - 1, ticks));
            return false;
        }
        self.armed = None;
        self.held = Some((owner, id, now.saturating_add(ticks)));
        true
    }
    pub fn withheld(&mut self, id: u64, now: u64) -> bool {
        let Some((_, active, deadline)) = self.held else {
            return false;
        };
        if id != active {
            return false;
        }
        if now < deadline {
            true
        } else {
            self.held = None;
            false
        }
    }
    pub fn status(&self) -> [u64; 8] {
        if let Some((owner, id, until)) = self.held {
            [1, owner, id, until, 0, 0, 0, 0]
        } else if let Some((owner, skip, ticks)) = self.armed {
            [0, owner, 0, 0, 1, skip, ticks, 0]
        } else {
            [0; 8]
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_selected_owner_mutations_count_and_retention_has_a_deadline() {
        let mut h = Hold::default();
        assert!(h.arm(2, 1, 50));
        assert!(!h.submitted(3, 1, true, 0));
        assert!(!h.submitted(2, 2, false, 0));
        assert!(!h.submitted(2, 3, true, 0));
        assert!(h.submitted(2, 4, true, 10));
        assert!(!h.arm(2, 0, 50));
        assert!(h.withheld(4, 59));
        assert!(!h.withheld(5, 59));
        assert!(!h.withheld(4, 60));
        assert_eq!(h.status(), [0; 8]);
    }
    #[test]
    fn invalid_diagnostics_cannot_replace_an_armed_hold() {
        let mut h = Hold::default();
        assert!(h.arm(2, 0, 50));
        let expected = h.status();
        for (p, s, t) in [(0, 0, 50), (2, 17, 50), (2, 0, 0), (2, 0, 501)] {
            assert!(!h.arm(p, s, t));
            assert_eq!(h.status(), expected);
        }
    }
}
