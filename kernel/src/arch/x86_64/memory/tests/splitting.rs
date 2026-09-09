// SPDX-License-Identifier: Apache-2.0
//! Structural tests for inherited huge leaves; synthetic roots are never activated.
use super::super::tables::{self, HUGE, NX, PRESENT, WRITE};
use super::Memory;
use rustic_kernel::memory::PagePermissions;

pub(super) fn verify(memory: &mut Memory) {
    let physical = &mut memory.physical;
    let before = physical.frames.free_count();
    let root = physical.allocate_zeroed().unwrap();
    let pdpt = physical.allocate_zeroed().unwrap();
    physical.write(root, 256, pdpt | PRESENT | WRITE);
    physical.write(pdpt, 0, PRESENT | WRITE | HUGE | NX | (1 << 12));
    let directory = tables::split(physical, pdpt, 0, 3).unwrap();
    let leaf = tables::split(physical, directory, 2, 2).unwrap();
    let address = 0xffff_8000_0040_1000;
    let mapping = tables::lookup(physical, root, address).unwrap();
    assert_eq!(mapping.physical, 0x40_1000);
    assert!(mapping.writable && !mapping.executable && !mapping.user);
    assert_ne!(physical.read(leaf, 1) & (1 << 7), 0); // PAT moved from bit 12 to 7.
    tables::protect(physical, root, address, Some(PagePermissions::READ_ONLY)).unwrap();
    assert!(!tables::lookup(physical, root, address).unwrap().writable);
    assert!(
        tables::lookup(physical, root, address + 4096)
            .unwrap()
            .writable
    );
    assert_ne!(physical.read(leaf, 1) & (1 << 7), 0);
    for frame in [leaf, directory, pdpt, root] {
        physical.release(frame).unwrap();
    }
    assert_eq!(physical.frames.free_count(), before);
}
