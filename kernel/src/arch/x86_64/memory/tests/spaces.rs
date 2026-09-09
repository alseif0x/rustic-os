// SPDX-License-Identifier: Apache-2.0
use super::super::{Error, bootstrap};
use super::{Memory, page, read, write};
use rustic_kernel::memory::PagePermissions;

pub(super) fn verify(memory: &mut Memory) {
    let before = memory.physical.frames.free_count();
    let mut first = memory.kernel.child(&mut memory.physical).unwrap();
    let mut second = memory.kernel.child(&mut memory.physical).unwrap();
    let access = PagePermissions {
        user: true,
        ..PagePermissions::READ_WRITE
    };
    first
        .map_zeroed(&mut memory.physical, page(), access)
        .unwrap();
    second
        .map_zeroed(&mut memory.physical, page(), access)
        .unwrap();
    let one = first.lookup(&memory.physical, page().address()).unwrap();
    let two = second.lookup(&memory.physical, page().address()).unwrap();
    assert_ne!(one.physical, two.physical);
    assert!(one.user && one.writable && !one.executable);
    assert!(
        !first
            .lookup(&memory.physical, bootstrap::text_start())
            .unwrap()
            .user
    );
    // SAFETY: Both roots share identical upper kernel, stack and IRQ mappings.
    // Only scalar values survive switches; no references into the lower half exist.
    unsafe {
        first.activate().unwrap();
        assert_eq!(read(page().address()), 0);
        write(page().address(), 0xaaaa);
        assert_eq!(first.destroy(&mut memory.physical), Err(Error::ActiveSpace));
        second.activate().unwrap();
        assert_eq!(read(page().address()), 0);
        write(page().address(), 0xbbbb);
        first.activate().unwrap();
        assert_eq!(read(page().address()), 0xaaaa);
        second.activate().unwrap();
        assert_eq!(read(page().address()), 0xbbbb);
        memory.kernel.activate().unwrap();
    }
    first.destroy(&mut memory.physical).unwrap();
    second.destroy(&mut memory.physical).unwrap();
    assert_eq!(
        first.destroy(&mut memory.physical),
        Err(Error::InvalidAddress)
    );
    assert_eq!(memory.physical.frames.free_count(), before);
}
