// SPDX-License-Identifier: Apache-2.0
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deadline(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overflow;

impl Deadline {
    pub fn after(now: u64, ticks: u64) -> Result<Self, Overflow> {
        now.checked_add(ticks).map(Self).ok_or(Overflow)
    }

    pub fn reached(self, now: u64) -> bool {
        now >= self.0
    }

    pub fn ticks(self) -> u64 {
        self.0
    }
}

/// Convert a rational timer period without cumulative rounding error. Saturates.
pub fn ticks_to_nanos(ticks: u64, divisor: u16, input_hz: u32) -> Option<u64> {
    if input_hz == 0 || divisor == 0 {
        return None;
    }
    let nanos = u128::from(ticks) * u128::from(divisor) * 1_000_000_000 / u128::from(input_hz);
    Some(nanos.min(u128::from(u64::MAX)) as u64)
}
