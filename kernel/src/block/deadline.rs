// SPDX-License-Identifier: Apache-2.0
//! Device latency uses elapsed ticks; a stalled clock has a separate polling guard.
const REQUEST_TICKS: u64 = 500;
const STALLED_POLLS: u64 = 5_000_000;

pub struct RequestBudget {
    started: u64,
    observed_tick: u64,
    polls: u64,
    stalled_polls: u64,
}

impl RequestBudget {
    /// All observations must use the same nondecreasing, saturating tick source.
    pub fn new(now: u64) -> Self {
        Self {
            started: now,
            observed_tick: now,
            polls: 0,
            stalled_polls: 0,
        }
    }

    /// Call only after checking for an actual completion. Advancing time resets
    /// the stall guard; CPU speed must not shorten the device's elapsed budget.
    pub fn expired_pending(&mut self, now: u64) -> bool {
        self.polls = self.polls.saturating_add(1);
        if now == self.observed_tick {
            self.stalled_polls = self.stalled_polls.saturating_add(1);
        } else {
            self.observed_tick = now;
            self.stalled_polls = 0;
        }
        now.saturating_sub(self.started) >= REQUEST_TICKS || self.stalled_polls >= STALLED_POLLS
    }

    pub fn started(&self) -> u64 {
        self.started
    }

    pub fn polls(&self) -> u64 {
        self.polls
    }

    pub fn stalled_polls(&self) -> u64 {
        self.stalled_polls
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_completion_keeps_its_budget_until_the_elapsed_deadline() {
        let mut budget = RequestBudget::new(10);
        assert!(!budget.expired_pending(35)); // The old 250 ms limit.
        assert!(!budget.expired_pending(110));
        assert!(!budget.expired_pending(509));
        assert!(budget.expired_pending(510));
    }

    #[test]
    fn polling_speed_does_not_shorten_a_progressing_clock_budget() {
        let mut budget = RequestBudget::new(0);
        for tick in 1..=2 {
            for _ in 0..STALLED_POLLS - 1 {
                assert!(!budget.expired_pending(tick));
            }
        }
        assert!(budget.polls() > STALLED_POLLS);
        assert!(budget.stalled_polls() < STALLED_POLLS);
        assert!(!budget.expired_pending(REQUEST_TICKS - 1));
    }

    #[test]
    fn a_nonadvancing_clock_still_terminates_an_uncompleted_request() {
        let mut budget = RequestBudget::new(u64::MAX);
        for _ in 1..STALLED_POLLS {
            assert!(!budget.expired_pending(u64::MAX));
        }
        assert!(budget.expired_pending(u64::MAX));
        assert_eq!(budget.stalled_polls(), STALLED_POLLS);
    }

    #[test]
    fn tick_progress_restarts_the_entire_stall_guard_even_at_saturation() {
        let mut budget = RequestBudget::new(u64::MAX - 1);
        for _ in 1..STALLED_POLLS {
            assert!(!budget.expired_pending(u64::MAX - 1));
        }
        assert_eq!(budget.stalled_polls(), STALLED_POLLS - 1);
        assert!(!budget.expired_pending(u64::MAX));
        assert_eq!(budget.stalled_polls(), 0);
        for _ in 1..STALLED_POLLS {
            assert!(!budget.expired_pending(u64::MAX));
        }
        assert!(budget.expired_pending(u64::MAX));
    }
}
