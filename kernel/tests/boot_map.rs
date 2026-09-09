// SPDX-License-Identifier: Apache-2.0
use rustic_kernel::boot::{BootMode, MapError, validate_map};

#[test]
fn rejects_empty_reserved_only_and_corrupt_maps() {
    assert_eq!(validate_map([]), Err(MapError::NoUsableMemory));
    assert_eq!(
        validate_map([(0, 4096, false)]),
        Err(MapError::NoUsableMemory)
    );
    assert_eq!(
        validate_map([(u64::MAX, 1, true)]),
        Err(MapError::InvalidRange)
    );
    assert_eq!(validate_map([(0, 0, true)]), Err(MapError::InvalidRange));
    assert_eq!(
        validate_map([(4096, 4096, true), (4095, 1, false)]),
        Err(MapError::UnorderedOrOverlapping)
    );
}

#[test]
fn counts_only_usable_memory_and_bounds_work() {
    let summary = validate_map([(0, 4096, false), (4096, 8192, true)]).unwrap();
    assert_eq!(summary.usable_bytes, 8192);
    assert_eq!(summary.entries, 2);
    assert_eq!(
        validate_map((0..4097).map(|n| (n * 4096, 4096, true))),
        Err(MapError::TooManyEntries)
    );
}

#[test]
fn unknown_or_ambiguous_modes_cannot_report_success() {
    assert_eq!(BootMode::parse(b"mode=ok"), Some(BootMode::Ok));
    assert_eq!(BootMode::parse(b"mode=panic"), Some(BootMode::Panic));
    assert_eq!(BootMode::parse(b"mode=hang"), Some(BootMode::Hang));
    assert_eq!(BootMode::parse(b""), None);
    assert_eq!(BootMode::parse(b"mode=ok mode=panic"), None);
    assert_eq!(BootMode::parse(b"mode=\xff"), None);
}
