// SPDX-License-Identifier: Apache-2.0
//! Behaviour of the bounded region allocator that the guest heap runs.
//!
//! These are host tests over a plain byte buffer: they exercise the same policy
//! code as RusticOS, but they demonstrate nothing about mapping, page
//! permissions or reclamation. Guest evidence comes from the kernel probe
//! fixture described in `docs/SDK.md`.
use core::ptr::NonNull;
use rustic_sdk::memory::{AllocError, Allocator, Stats};

/// Page-aligned so that a 4096-byte alignment request is reachable and the
/// usable capacity is exactly the buffer length.
#[repr(C, align(4096))]
struct Arena([u8; 16384]);

impl Arena {
    fn new() -> Self {
        Self([0; 16384])
    }
}

/// Payload of a block that occupies exactly `SLOT` bytes with its header.
const SLOT: usize = 128;

fn payload() -> usize {
    SLOT - Allocator::OVERHEAD
}

fn shape(stats: Stats) -> (usize, usize, usize, usize, usize) {
    (
        stats.capacity,
        stats.used,
        stats.free,
        stats.largest_free,
        stats.blocks,
    )
}

/// Fills the region with equally sized blocks until no free byte is left.
fn fill(allocator: &mut Allocator<'_>) -> Vec<rustic_sdk::memory::Block> {
    let mut blocks = Vec::new();
    while let Ok(block) = allocator.alloc(payload(), 8) {
        blocks.push(block);
    }
    assert_eq!(allocator.free_bytes(), 0);
    assert_eq!(allocator.alloc(1, 1), Err(AllocError::Empty));
    blocks
}

#[test]
fn requested_alignments_are_honoured_and_fully_returned() {
    let mut arena = Arena::new();
    let mut allocator = Allocator::new(&mut arena.0).unwrap();
    let initial = shape(allocator.stats());
    for align in [1, 8, 64, 4096] {
        let first = allocator.alloc(100, align).unwrap();
        let second = allocator.alloc(3000, align).unwrap();
        assert_eq!(first.address() % align, 0, "first at {align}");
        assert_eq!(second.address() % align, 0, "second at {align}");
        assert_ne!(first.address(), second.address());
        // Alignment padding is accounted for, never silently leaked.
        assert_eq!(
            allocator.used_bytes() + allocator.free_bytes(),
            allocator.capacity()
        );
        allocator.free(second).unwrap();
        allocator.free(first).unwrap();
        assert_eq!(shape(allocator.stats()), initial, "after {align}");
    }
    assert_eq!(allocator.alloc(100, 8192), Err(AllocError::Request));
}

#[test]
fn a_freed_block_is_reused_by_a_same_size_request() {
    let mut arena = Arena::new();
    let mut allocator = Allocator::new(&mut arena.0).unwrap();
    let blocks = fill(&mut allocator);
    let victim = blocks[10];
    let occupied = allocator.stats();
    allocator.free(victim).unwrap();
    assert_eq!(allocator.free_bytes(), SLOT);
    assert_eq!(allocator.blocks(), occupied.blocks - 1);
    let again = allocator.alloc(payload(), 8).unwrap();
    assert_eq!(again.address(), victim.address());
    assert_eq!(shape(allocator.stats()), shape(occupied));
}

#[test]
fn coalescing_two_neighbours_admits_a_request_neither_could_hold() {
    let mut arena = Arena::new();
    let mut allocator = Allocator::new(&mut arena.0).unwrap();
    let blocks = fill(&mut allocator);
    let wide = 2 * SLOT - Allocator::OVERHEAD;
    allocator.free(blocks[10]).unwrap();
    assert_eq!(allocator.largest_free(), SLOT);
    assert_eq!(allocator.alloc(wide, 8), Err(AllocError::Exhausted));
    allocator.free(blocks[11]).unwrap();
    assert_eq!(allocator.largest_free(), 2 * SLOT);
    let merged = allocator.alloc(wide, 8).unwrap();
    assert_eq!(merged.address(), blocks[10].address());
    assert_eq!(allocator.free_bytes(), 0);
}

#[test]
fn fragmentation_reports_exhausted_while_free_bytes_remain() {
    let mut arena = Arena::new();
    let mut allocator = Allocator::new(&mut arena.0).unwrap();
    let blocks = fill(&mut allocator);
    allocator.free(blocks[10]).unwrap();
    allocator.free(blocks[40]).unwrap();
    let wide = 2 * SLOT - Allocator::OVERHEAD;
    let stats = allocator.stats();
    assert_eq!(stats.free, 2 * SLOT);
    assert_eq!(stats.largest_free, SLOT);
    // Enough free bytes in total, no run that fits: distinct from `Empty`.
    assert_eq!(allocator.alloc(wide, 8), Err(AllocError::Exhausted));
    assert_eq!(allocator.stats(), stats);
}

#[test]
fn double_free_stale_and_foreign_handles_are_rejected_without_accounting_drift() {
    let mut arena = Arena::new();
    let mut other = Arena::new();
    let mut allocator = Allocator::new(&mut arena.0).unwrap();
    let mut foreign = Allocator::new(&mut other.0).unwrap();
    let blocks = fill(&mut allocator);
    let victim = blocks[10];
    allocator.free(victim).unwrap();
    let after_free = allocator.stats();

    assert_eq!(allocator.free(victim), Err(AllocError::DoubleFree));
    assert_eq!(allocator.bytes(&victim), Err(AllocError::DoubleFree));
    assert_eq!(allocator.stats(), after_free);

    let outsider = foreign.alloc(payload(), 8).unwrap();
    assert_eq!(allocator.free(outsider), Err(AllocError::Unowned));
    assert_eq!(allocator.stats(), after_free);

    // A stale handle whose address is live again but too large for the block
    // that now starts there is not accepted as that block.
    let small = payload() / 4;
    let replacement = allocator.alloc(small, 8).unwrap();
    assert_eq!(replacement.address(), victim.address());
    assert_eq!(allocator.free(victim), Err(AllocError::Unowned));
    assert_eq!(allocator.bytes(&victim), Err(AllocError::Unowned));
    allocator.free(replacement).unwrap();
    assert_eq!(shape(allocator.stats()), shape(after_free));
}

#[test]
fn accounting_returns_to_the_initial_state_after_a_mixed_workload() {
    let mut arena = Arena::new();
    let mut allocator = Allocator::new(&mut arena.0).unwrap();
    let initial = shape(allocator.stats());
    let mut blocks = Vec::new();
    for index in 0..12 {
        blocks.push(allocator.alloc(17 * (index + 1), 1 << (index % 5)).unwrap());
    }
    for (index, block) in blocks.iter().enumerate() {
        allocator.bytes_mut(block).unwrap().fill(index as u8);
    }
    // Interleaved release must not disturb the surviving blocks' contents.
    for block in blocks.iter().step_by(2) {
        allocator.free(*block).unwrap();
    }
    for (index, block) in blocks.iter().enumerate().skip(1).step_by(2) {
        assert!(
            allocator
                .bytes(block)
                .unwrap()
                .iter()
                .all(|b| *b == index as u8)
        );
    }
    for block in blocks.iter().skip(1).step_by(2) {
        allocator.free(*block).unwrap();
    }
    assert_eq!(shape(allocator.stats()), initial);
    assert!(allocator.peak_used_bytes() > 0);
}

#[test]
fn zero_sized_and_oversized_requests_are_refused() {
    let mut arena = Arena::new();
    let mut allocator = Allocator::new(&mut arena.0).unwrap();
    let capacity = allocator.capacity();
    assert_eq!(allocator.alloc(0, 8), Err(AllocError::Request));
    assert_eq!(allocator.alloc(64, 0), Err(AllocError::Request));
    assert_eq!(allocator.alloc(64, 24), Err(AllocError::Request));
    assert_eq!(allocator.alloc(64, 8192), Err(AllocError::Request));
    // The region is entirely free, so an oversize request is a missing run.
    assert_eq!(allocator.alloc(capacity, 8), Err(AllocError::Exhausted));
    assert_eq!(allocator.alloc(usize::MAX, 8), Err(AllocError::Request));
    assert_eq!(allocator.free_bytes(), capacity);
    assert_eq!(allocator.blocks(), 0);
    let whole = allocator.alloc(capacity - Allocator::OVERHEAD, 1).unwrap();
    assert_eq!(allocator.free_bytes(), 0);
    assert_eq!(allocator.alloc(1, 1), Err(AllocError::Empty));
    allocator.free(whole).unwrap();
    assert_eq!(allocator.free_bytes(), capacity);
}

#[test]
fn extending_and_truncating_preserve_live_blocks_and_never_touch_them() {
    let mut arena = Arena::new();
    let base = NonNull::new(arena.0.as_mut_ptr()).unwrap();
    // SAFETY: `arena` owns 16384 initialized bytes and outlives `allocator`,
    // which is the only manager of that range; the allocator starts with the
    // first half and is extended into the second half of the same object.
    let mut allocator = unsafe { Allocator::from_raw(base, 8192) }.unwrap();
    assert_eq!(allocator.capacity(), 8192);
    let live = allocator.alloc(100, 8).unwrap();
    allocator.bytes_mut(&live).unwrap().fill(0x5a);
    assert_eq!(allocator.alloc(12000, 8), Err(AllocError::Exhausted));

    // SAFETY: the second half of the same `arena` is owned, initialized and
    // immediately follows the region, and no other reference reaches it.
    unsafe { allocator.extend(8192) }.unwrap();
    assert_eq!(allocator.capacity(), 16384);
    let wide = allocator.alloc(12000, 8).unwrap();
    assert!(allocator.bytes(&live).unwrap().iter().all(|b| *b == 0x5a));

    allocator.free(wide).unwrap();
    let target = allocator.truncation_target(4096);
    assert_eq!(target, 4096);
    allocator.truncate(target).unwrap();
    assert_eq!(allocator.capacity(), 4096);
    assert_eq!(allocator.truncation_target(4096), 4096);
    assert!(allocator.bytes(&live).unwrap().iter().all(|b| *b == 0x5a));

    // A live block blocks nothing beyond it, but the region cannot shrink past
    // it; only after the release can every page be given back.
    allocator.free(live).unwrap();
    assert_eq!(allocator.truncation_target(4096), 0);
    allocator.truncate(0).unwrap();
    assert_eq!(allocator.capacity(), 0);
    assert_eq!(allocator.alloc(1, 1), Err(AllocError::Empty));
    assert_eq!(allocator.truncate(4096), Err(AllocError::Region));
}

#[test]
fn a_stale_handle_is_refused_after_its_address_is_allocated_again() {
    let mut arena = Arena::new();
    let mut allocator = Allocator::new(&mut arena.0).unwrap();
    let blocks = fill(&mut allocator);
    let stale = blocks[10];
    allocator.free(stale).unwrap();
    let after_free = allocator.stats();

    // Same address and same size as the released allocation: only the
    // generation the allocator records distinguishes the two handles.
    let fresh = allocator.alloc(payload(), 8).unwrap();
    assert_eq!(fresh.address(), stale.address());
    assert_eq!(fresh.size(), stale.size());
    assert_ne!(fresh, stale);
    let occupied = allocator.stats();
    assert_eq!(allocator.free(stale), Err(AllocError::Unowned));
    assert_eq!(allocator.bytes(&stale), Err(AllocError::Unowned));
    assert_eq!(allocator.bytes_mut(&stale), Err(AllocError::Unowned));
    assert_eq!(allocator.stats(), occupied);

    // The same holds when the address is reused by a smaller allocation.
    allocator.free(fresh).unwrap();
    let smaller = allocator.alloc(payload() / 4, 8).unwrap();
    assert_eq!(smaller.address(), stale.address());
    let split = allocator.stats();
    assert_eq!(allocator.free(stale), Err(AllocError::Unowned));
    assert_eq!(allocator.bytes(&stale), Err(AllocError::Unowned));
    assert_eq!(allocator.stats(), split);

    // The handle the allocator actually issued is still accepted, and
    // releasing it returns the region to the state before the reuse.
    allocator.bytes_mut(&smaller).unwrap().fill(0x7e);
    assert!(
        allocator
            .bytes(&smaller)
            .unwrap()
            .iter()
            .all(|b| *b == 0x7e)
    );
    allocator.free(smaller).unwrap();
    assert_eq!(shape(allocator.stats()), shape(after_free));
}

#[test]
fn truncating_to_a_misaligned_length_is_refused_without_touching_the_region() {
    let mut arena = Arena::new();
    let base = NonNull::new(arena.0.as_mut_ptr()).unwrap();
    // SAFETY: `arena` owns 16384 initialized bytes, outlives `allocator` and is
    // managed by no one else; the allocator takes the whole object.
    let mut allocator = unsafe { Allocator::from_raw(base, 16384) }.unwrap();
    let live = allocator.alloc(64, 8).unwrap();
    allocator.bytes_mut(&live).unwrap().fill(0x33);
    let before = shape(allocator.stats());

    // 8196 is not a multiple of the block alignment. Accepting it would leave
    // the region unable to carry a header at its own end, which a later
    // extension could only discover after growing the region.
    assert_eq!(allocator.truncate(8196), Err(AllocError::Region));
    assert_eq!(shape(allocator.stats()), before);
    assert_eq!(allocator.capacity(), 16384);

    // The aligned neighbour of the same length is accepted, so the refusal is
    // about the invariant and not about the size of the request.
    allocator.truncate(8192).unwrap();
    assert_eq!(allocator.capacity(), 8192);
    assert_eq!(
        allocator.used_bytes() + allocator.free_bytes(),
        allocator.capacity()
    );
    assert!(allocator.bytes(&live).unwrap().iter().all(|b| *b == 0x33));
}

#[test]
fn extending_past_a_used_tail_adds_a_free_block_and_keeps_live_bytes() {
    let mut arena = Arena::new();
    let base = NonNull::new(arena.0.as_mut_ptr()).unwrap();
    // SAFETY: `arena` owns 16384 initialized bytes and outlives `allocator`,
    // which manages the first 4096 alone and is extended into the same object.
    let mut allocator = unsafe { Allocator::from_raw(base, 4096) }.unwrap();
    // One allocation spanning the whole region, so the region ends in a used
    // block and the new range cannot be merged into anything.
    let live = allocator.alloc(4096 - Allocator::OVERHEAD, 1).unwrap();
    allocator.bytes_mut(&live).unwrap().fill(0xa5);
    assert_eq!(allocator.free_bytes(), 0);

    // SAFETY: the bytes after the region belong to the same owned `arena` and
    // are initialized; no other reference reaches them.
    unsafe { allocator.extend(4096) }.unwrap();
    assert_eq!(allocator.capacity(), 8192);
    assert_eq!(allocator.free_bytes(), 4096);
    assert_eq!(allocator.largest_free(), 4096);
    assert_eq!(allocator.blocks(), 1);
    assert!(allocator.bytes(&live).unwrap().iter().all(|b| *b == 0xa5));

    // The appended range is one ordinary free block: it satisfies exactly one
    // allocation of its size and leaves the live block untouched.
    let second = allocator.alloc(4096 - Allocator::OVERHEAD, 1).unwrap();
    assert_eq!(second.address(), live.address() + 4096);
    assert_eq!(allocator.free_bytes(), 0);
    assert!(allocator.bytes(&live).unwrap().iter().all(|b| *b == 0xa5));
    allocator.free(second).unwrap();
    allocator.free(live).unwrap();
    assert_eq!(allocator.largest_free(), 8192);
}

#[test]
fn extending_a_region_truncated_to_nothing_makes_it_usable_again() {
    let mut arena = Arena::new();
    let base = NonNull::new(arena.0.as_mut_ptr()).unwrap();
    // SAFETY: as above; `allocator` is the only manager of this owned object.
    let mut allocator = unsafe { Allocator::from_raw(base, 4096) }.unwrap();
    let block = allocator.alloc(100, 8).unwrap();
    allocator.free(block).unwrap();
    allocator.truncate(0).unwrap();
    assert_eq!(allocator.capacity(), 0);
    assert_eq!(allocator.free_bytes(), 0);
    assert_eq!(allocator.alloc(1, 1), Err(AllocError::Empty));

    // Nothing is left to attach the new range to, so it becomes the region's
    // first block; the extension itself cannot fail for lack of bookkeeping.
    // SAFETY: the first 8192 bytes of `arena` are owned, initialized and start
    // at the region's address; no other reference reaches them.
    unsafe { allocator.extend(8192) }.unwrap();
    assert_eq!(
        shape(allocator.stats()),
        (8192, 0, 8192, 8192, 0),
        "one free block spanning the whole region"
    );
    let wide = allocator.alloc(8192 - Allocator::OVERHEAD, 8).unwrap();
    assert_eq!(wide.address(), allocator.address() + Allocator::OVERHEAD);
    allocator.bytes_mut(&wide).unwrap().fill(0xc7);
    assert!(allocator.bytes(&wide).unwrap().iter().all(|b| *b == 0xc7));
    allocator.free(wide).unwrap();
    assert_eq!(allocator.free_bytes(), 8192);
    // A misaligned extension is still refused, region or not.
    // SAFETY: the same owned and initialized bytes as above.
    assert_eq!(unsafe { allocator.extend(12) }, Err(AllocError::Region));
    assert_eq!(allocator.capacity(), 8192);
}
