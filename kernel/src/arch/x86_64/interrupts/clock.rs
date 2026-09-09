// SPDX-License-Identifier: Apache-2.0
use super::pit;
use core::sync::atomic::{AtomicU64, Ordering};
use rustic_kernel::time::ticks_to_nanos;

static TICKS: AtomicU64 = AtomicU64::new(0);

pub(super) fn advance() {
    // Saturation preserves monotonicity even at the representational limit.
    let _ = TICKS.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
        Some(n.saturating_add(1))
    });
}

pub(crate) fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub(super) fn nanos() -> u64 {
    ticks_to_nanos(ticks(), pit::DIVISOR, pit::INPUT_HZ).expect("fixed nonzero PIT parameters")
}
