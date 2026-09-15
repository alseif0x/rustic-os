// SPDX-License-Identifier: Apache-2.0
//! Budget exhaustion observed from a product child, not from a kernel fixture.
//!
//! This step exists so the owner can see the per-process page budget refuse a
//! growth and then see the pages come back, through the ordinary shell control
//! path. It owns nothing that survives the call: it reserves a private heap,
//! grows it one page at a time until the kernel answers `Full`, drops it and
//! reports what the kernel says afterwards. The collection state of
//! [`super::state`] is never read or written here, so a stress run cannot
//! disturb a candidate the owner is still assembling.
//!
//! Reply words, as observed through `ACT_STATUS`:
//!
//! | Word | Meaning |
//! | --- | --- |
//! | 0 | `0` on success, `106` when the budget was not reached or not released |
//! | 1 | Peak pages this process held at once during the run |
//! | 2 | `1` when a growth was refused with `Full` |
//! | 3 | Pages still mapped after the heap was dropped |
//! | 4 | The per-process page limit the kernel reported |
//!
//! Word 3 is the whole process, not this heap: a child may already hold pages
//! of its own, so success requires it to equal the count taken before the run
//! rather than zero.
use rustic_sdk::abi::runtime;
use rustic_sdk::memory::{
    Error, Heap,
    raw::{self, Query},
};

/// This client's refusal code for a budget that behaved unexpectedly: the
/// same code the collection reports for a memory refusal.
fn memory() -> u64 {
    super::report::code(super::state::Fault::Memory)
}

/// Upper bound on growth steps, so an implausible limit ends the run instead of
/// spending the owner's exchange deadline. A refusal is expected well inside it.
const STEPS: u64 = 128;

/// Grows a private heap to the per-process budget and releases it again.
pub(super) fn run() -> [u64; 8] {
    match stress() {
        Ok(reply) => reply,
        // A query, a reservation or an unexpected map refusal leaves nothing to
        // report but the refusal itself; the heap is already dropped.
        Err(_) => [memory(), 0, 0, 0, 0, 0, 0, 0],
    }
}

fn stress() -> Result<[u64; 8], Error> {
    let limit = raw::query(Query::PageLimit)?;
    let before = raw::query(Query::MappedPages)?;
    let (peak, full) = exhaust()?;
    let after = raw::query(Query::MappedPages)?;
    // `Full` also answers physical frame exhaustion, so only a peak that reached
    // the per-process budget proves the budget itself refused.
    let released = full && peak == limit && after == before;
    Ok([
        if released { 0 } else { memory() },
        peak,
        u64::from(full),
        after,
        limit,
        0,
        0,
        0,
    ])
}

/// Reserves one page, grows it until the budget refuses, and reports the peak.
///
/// The heap is dropped before this returns, so the caller's next query observes
/// the released state.
fn exhaust() -> Result<(u64, bool), Error> {
    let mut heap = Heap::reserve(1, true)?;
    let mut peak = raw::query(Query::MappedPages)?;
    for _ in 0..STEPS {
        match heap.grow(1) {
            Ok(()) => {
                let mapped = raw::query(Query::MappedPages)?;
                if mapped > peak {
                    peak = mapped;
                }
            }
            // The budget, not an accident: the heap is untouched and still owns
            // everything it mapped, which `Drop` then releases.
            Err(Error::Map(runtime::Error::Full)) => return Ok((peak, true)),
            Err(other) => return Err(other),
        }
    }
    Ok((peak, false))
}
