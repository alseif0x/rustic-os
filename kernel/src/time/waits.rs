// SPDX-License-Identifier: Apache-2.0
use super::Deadline;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitError {
    InvalidSlot,
    Occupied,
}

/// One owner registers simultaneous deadlines. No scheduler or shared mutable state.
pub struct WaitSet<const N: usize> {
    slots: [Option<Deadline>; N],
}

impl<const N: usize> Default for WaitSet<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> WaitSet<N> {
    pub const fn new() -> Self {
        Self { slots: [None; N] }
    }

    pub fn register(&mut self, slot: usize, deadline: Deadline) -> Result<(), WaitError> {
        let entry = self.slots.get_mut(slot).ok_or(WaitError::InvalidSlot)?;
        if entry.is_some() {
            return Err(WaitError::Occupied);
        }
        *entry = Some(deadline);
        Ok(())
    }

    pub fn cancel(&mut self, slot: usize) -> Result<bool, WaitError> {
        Ok(self
            .slots
            .get_mut(slot)
            .ok_or(WaitError::InvalidSlot)?
            .take()
            .is_some())
    }

    pub fn next_deadline(&self) -> Option<Deadline> {
        self.slots
            .iter()
            .flatten()
            .copied()
            .min_by_key(|d| d.ticks())
    }

    /// Complete each due registration exactly once, including equal deadlines.
    pub fn complete(&mut self, now: u64) -> [bool; N] {
        core::array::from_fn(|index| {
            if self.slots[index].is_some_and(|deadline| deadline.reached(now)) {
                self.slots[index] = None;
                true
            } else {
                false
            }
        })
    }
}
