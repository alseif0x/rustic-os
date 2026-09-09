// SPDX-License-Identifier: Apache-2.0
use rustic_kernel::memory::{FrameAllocator, FrameError};
#[test]
fn contiguous_allocation_crosses_bitmap_words_and_recovers() {
    let mut managed = [0; 2];
    let mut allocated = [0; 2];
    let mut frames = FrameAllocator::new(&mut managed, &mut allocated).unwrap();
    frames
        .import([(62 * 4096, 8 * 4096, true)].into_iter())
        .unwrap();
    let a = frames.allocate_contiguous(4).unwrap();
    assert_eq!(a, 62 * 4096);
    let b = frames.allocate_contiguous(4).unwrap();
    assert_eq!(b, 66 * 4096);
    assert_eq!(frames.allocate_contiguous(1), Err(FrameError::Exhausted));
    for start in [a, b] {
        for i in 0..4 {
            frames.release(start + i * 4096).unwrap();
        }
    }
    assert_eq!(frames.free_count(), 8);
}
#[test]
fn fragmented_failure_is_atomic_and_does_not_cross_reserved_pages() {
    let mut managed = [0; 1];
    let mut allocated = [0; 1];
    let mut frames = FrameAllocator::new(&mut managed, &mut allocated).unwrap();
    frames.import([(0, 8 * 4096, true)].into_iter()).unwrap();
    for i in [1, 3, 5, 7] {
        frames.reserve(i * 4096, 4096).unwrap();
    }
    assert_eq!(frames.allocate_contiguous(2), Err(FrameError::Exhausted));
    assert_eq!(frames.allocate_contiguous(0), Err(FrameError::InvalidRange));
    assert_eq!(frames.allocate_contiguous(5), Err(FrameError::InvalidRange));
    assert_eq!(frames.free_count(), 4);
    for i in [0, 2, 4, 6] {
        assert_eq!(frames.allocate().unwrap(), i * 4096);
    }
}
