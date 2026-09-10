// SPDX-License-Identifier: Apache-2.0
//! One in-flight exchange per endpoint. Abandonment requires a fresh binding.
#[derive(Default)]
pub struct State {
    next: u64,
    active: Option<u64>,
    failed: bool,
}
impl State {
    pub fn ticket(&self) -> Option<u64> {
        if self.failed || self.active.is_some() {
            None
        } else {
            self.next.checked_add(1)
        }
    }
    pub fn sent(&mut self, ticket: u64) -> bool {
        if self.ticket() != Some(ticket) {
            return false;
        }
        self.next = ticket;
        self.active = Some(ticket);
        true
    }
    pub fn accept(&mut self, ticket: u64) -> bool {
        if self.failed || self.active != Some(ticket) {
            self.fail();
            return false;
        }
        self.active = None;
        true
    }
    pub fn pending(&self) -> bool {
        self.active.is_some() && !self.failed
    }
    pub fn failed(&self) -> bool {
        self.failed
    }
    pub fn fail(&mut self) {
        self.failed = true;
        self.active = None;
    }
}
#[cfg(test)]
mod tests {
    use super::State;
    #[test]
    fn unsent_backpressure_does_not_admit_or_duplicate_an_exchange() {
        let mut state = State::default();
        assert_eq!(state.ticket(), Some(1));
        assert_eq!(state.ticket(), Some(1));
        assert!(state.sent(1));
        assert_eq!(state.ticket(), None);
        assert!(!state.sent(1));
        assert!(state.accept(1));
        assert_eq!(state.ticket(), Some(2));
    }
    #[test]
    fn abandoned_or_mismatched_responses_cannot_complete_another_call() {
        let mut state = State::default();
        assert!(state.sent(1));
        state.fail();
        assert!(!state.accept(1));
        assert_eq!(state.ticket(), None);
        let mut fresh = State::default();
        assert!(fresh.sent(1));
        assert!(fresh.accept(1));
        assert!(fresh.sent(2));
        assert!(!fresh.accept(1));
        assert!(fresh.failed());
    }
    #[test]
    fn a_pending_exchange_can_accept_a_late_response_without_resubmission() {
        let mut state = State::default();
        assert!(state.sent(1));
        for _ in 0..10 {
            assert!(state.pending());
            assert_eq!(state.ticket(), None);
        }
        assert!(state.accept(1));
        assert!(!state.pending());
        assert_eq!(state.ticket(), Some(2));
    }
}
