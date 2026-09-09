// SPDX-License-Identifier: Apache-2.0
use rustic_kernel::memory::{
    FrameAllocator, FrameError, PAGE_SIZE, PagePermissions, VirtualPage, canonical,
};

#[test]
fn only_whole_usable_unreserved_frames_are_allocated() {
    let (mut managed, mut allocated) = ([0; 2], [0; 2]);
    let mut frames = FrameAllocator::new(&mut managed, &mut allocated).unwrap();
    frames
        .import([(0, 4096, false), (4097, 16383, true), (20480, 4096, false)].into_iter())
        .unwrap();
    frames.reserve(12289, 1).unwrap();
    assert_eq!(frames.total_count(), 2);
    assert_eq!(frames.allocate(), Ok(8192));
    assert_eq!(frames.allocate(), Ok(16384));
    assert_eq!(frames.allocate(), Err(FrameError::Exhausted));
    assert_eq!(frames.release(0), Err(FrameError::NotManaged));
    assert_eq!(frames.reserve(4096, 16384), Err(FrameError::InUse));
    frames.release(8192).unwrap();
    assert_eq!(frames.release(8192), Err(FrameError::NotAllocated));
    assert_eq!(frames.allocate(), Ok(8192));
}

#[test]
fn map_failures_do_not_partially_import_memory() {
    for entries in [
        vec![(0, 8192, true), (4096, 4096, true)],
        vec![(0, 4096, true), (u64::MAX, 4096, true)],
        vec![(0, 4096, true), (1024 * 1024, 4096, true)],
    ] {
        let (mut managed, mut allocated) = ([0; 1], [0; 1]);
        let mut frames = FrameAllocator::new(&mut managed, &mut allocated).unwrap();
        assert!(frames.import(entries.into_iter()).is_err());
        assert_eq!(frames.free_count(), 0);
    }
}

#[test]
fn exhausted_storage_can_be_freed_and_reused_without_duplicates() {
    let (mut managed, mut allocated) = ([0; 3], [0; 3]);
    let mut frames = FrameAllocator::new(&mut managed, &mut allocated).unwrap();
    frames
        .import([(0, 192 * PAGE_SIZE, true)].into_iter())
        .unwrap();
    frames.reserve(0, PAGE_SIZE).unwrap();
    let mut addresses = Vec::new();
    while let Ok(address) = frames.allocate() {
        addresses.push(address);
    }
    assert_eq!(addresses.len(), 191);
    addresses.sort_unstable();
    addresses.dedup();
    assert_eq!(addresses.len(), 191);
    for address in addresses {
        frames.release(address).unwrap();
    }
    assert_eq!(frames.free_count(), 191);
    assert_eq!(frames.release(3), Err(FrameError::InvalidRange));
    assert_eq!(frames.reserve(u64::MAX, 2), Err(FrameError::InvalidRange));
}

#[test]
fn page_policy_rejects_noncanonical_unaligned_null_upper_and_writable_code() {
    for address in [0, 1, 4097, 1 << 47, 0xffff_8000_0000_0000, u64::MAX] {
        assert!(VirtualPage::new(address).is_none());
    }
    assert!(VirtualPage::new(4096).is_some());
    assert!(!canonical(1 << 47));
    assert!(canonical(0xffff_8000_0000_0000));
    assert!(PagePermissions::CODE.valid());
    assert!(
        !PagePermissions {
            writable: true,
            executable: true,
            user: true
        }
        .valid()
    );
}
