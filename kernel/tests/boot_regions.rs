// SPDX-License-Identifier: Apache-2.0
use rustic_kernel::boot::{MemoryRegion, RegionError};

#[test]
fn rejects_empty_and_wrapping_firmware_ranges() {
    assert_eq!(MemoryRegion::new(0, 0), Err(RegionError::Empty));
    assert_eq!(
        MemoryRegion::new(u64::MAX - 3, 4),
        Err(RegionError::AddressOverflow)
    );
}

#[test]
fn uses_half_open_boundaries_without_losing_the_last_valid_byte() {
    let region = MemoryRegion::new(u64::MAX - 4, 4).unwrap();
    assert_eq!(region.end(), u64::MAX);
    assert!(region.contains(u64::MAX - 1));
    assert!(!region.contains(u64::MAX));
    assert!(!region.contains(region.start() - 1));
}

#[test]
fn distinguishes_adjacency_overlap_and_enclosure_symmetrically() {
    let base = MemoryRegion::new(4096, 4096).unwrap();
    for (start, length, expected) in [
        (0, 4096, false),
        (8192, 4096, false),
        (4095, 2, true),
        (8191, 2, true),
        (5000, 1, true),
        (0, 16384, true),
    ] {
        let other = MemoryRegion::new(start, length).unwrap();
        assert_eq!(base.overlaps(other), expected);
        assert_eq!(other.overlaps(base), expected);
    }
}
