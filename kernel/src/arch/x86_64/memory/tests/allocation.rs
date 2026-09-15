// SPDX-License-Identifier: Apache-2.0
use super::super::Error;
use super::{Memory, SCRATCH, page, read, write};
use rustic_kernel::memory::{FrameError, PAGE_SIZE, PagePermissions};

pub(crate) fn with_free_frames(
    memory: &mut Memory,
    remaining: usize,
    test: impl FnOnce(&mut Memory),
) {
    let initial = memory.free_frames();
    assert!(remaining < initial);
    let mut head = 0;
    while memory.free_frames() > remaining {
        let frame = memory.physical.frames.allocate().unwrap();
        memory.physical.write(frame, 0, head);
        head = frame;
    }
    test(memory);
    assert_eq!(
        memory.free_frames(),
        remaining,
        "failed operation leaked frames"
    );
    while head != 0 {
        let next = memory.physical.read(head, 0);
        memory.physical.release(head).unwrap();
        head = next;
    }
    assert_eq!(memory.free_frames(), initial);
}

pub(super) fn pages(memory: &mut Memory) {
    let before = memory.physical.frames.free_count();
    for _ in 0..16 {
        memory
            .kernel
            .map_zeroed(&mut memory.physical, page(), PagePermissions::READ_WRITE)
            .unwrap();
        assert_eq!(read(page().address()), 0);
        write(page().address(), 0x1234_5678_abcd_ef00);
        assert_eq!(read(page().address()), 0x1234_5678_abcd_ef00);
        assert_eq!(
            memory
                .kernel
                .map_zeroed(&mut memory.physical, page(), PagePermissions::READ_WRITE),
            Err(Error::AlreadyMapped)
        );
        memory.kernel.unmap(&mut memory.physical, page()).unwrap();
        assert!(
            memory
                .kernel
                .lookup(&memory.physical, page().address())
                .is_none()
        );
        assert_eq!(
            memory.kernel.unmap(&mut memory.physical, page()),
            Err(Error::NotMapped)
        );
        assert_eq!(memory.physical.frames.free_count(), before);
    }
    assert_eq!(
        memory.kernel.map_zeroed(
            &mut memory.physical,
            page(),
            PagePermissions {
                writable: true,
                executable: true,
                user: true
            }
        ),
        Err(Error::WritableExecutable)
    );
    assert_eq!(memory.physical.frames.free_count(), before);
}

/// Store the temporary ownership list in the acquired pages themselves, avoiding
/// a large stack array or heap while exhausting the actual machine's free pool.
pub(super) fn exhaust(memory: &mut Memory) -> usize {
    let initial = memory.physical.frames.free_count();
    let mut head = 0;
    let mut count = 0;
    loop {
        match memory.physical.frames.allocate() {
            Ok(frame) => {
                memory.physical.write(frame, 0, head);
                head = frame;
                count += 1;
            }
            Err(FrameError::Exhausted) => break,
            other => panic!("unexpected allocation: {other:?}"),
        }
    }
    assert_eq!(count, initial);
    assert!(count > 2);
    let next = memory.physical.read(head, 0);
    memory.physical.release(head).unwrap();
    let reused = memory.physical.allocate_zeroed().unwrap();
    assert_eq!(reused, head); // It was the only free frame.
    assert!((0..512).all(|i| memory.physical.read(reused, i) == 0));
    memory.physical.write(head, 0, next);
    // Two free frames cannot satisfy a data page plus three missing table levels.
    for _ in 0..2 {
        let next = memory.physical.read(head, 0);
        memory.physical.release(head).unwrap();
        head = next;
    }
    assert_eq!(
        memory
            .kernel
            .map_zeroed(&mut memory.physical, page(), PagePermissions::READ_WRITE),
        Err(Error::Frames(FrameError::Exhausted))
    );
    assert_eq!(memory.physical.frames.free_count(), 2);
    assert!(
        memory
            .kernel
            .lookup(&memory.physical, page().address())
            .is_none()
    );
    while head != 0 {
        let next = memory.physical.read(head, 0);
        memory.physical.release(head).unwrap();
        head = next;
    }
    assert_eq!(memory.physical.frames.free_count(), initial);
    count
}

/// A user run that runs out of frames halfway leaves nothing behind: the pages
/// mapped so far are unmapped, their frames and the tables built for them are
/// returned, and the space keeps the range completely unmapped.
pub(super) fn user_rollback(memory: &mut Memory) {
    const PAGES: u64 = 4;
    let before = memory.free_frames();
    let mut space = memory.create_user().unwrap();
    // Measure instead of assuming: the first run into an empty lower half pays
    // for the data pages and for every missing table level above them.
    let available = memory.free_frames();
    memory
        .map_user_pages(&mut space, SCRATCH, PAGES, true)
        .unwrap();
    let cost = available - memory.free_frames();
    assert!(cost > PAGES as usize, "tables were already present");
    memory.unmap_user_pages(&mut space, SCRATCH, PAGES).unwrap();
    assert_eq!(memory.free_frames(), available);

    // One frame short of the whole run: the call fails and gives everything it
    // took back, which `with_free_frames` checks for the budget as a whole.
    with_free_frames(memory, cost - 1, |memory| {
        assert_eq!(
            memory.map_user_pages(&mut space, SCRATCH, PAGES, true),
            Err(Error::Frames(FrameError::Exhausted))
        );
        assert!(
            (0..PAGES).all(|index| space
                .inner
                .lookup(&memory.physical, SCRATCH + index * PAGE_SIZE)
                .is_none()),
            "a page of the failed run stayed mapped"
        );
    });

    // Exactly enough frames: the same run succeeds, spends all of them, and
    // unmapping returns the tables as well.
    with_free_frames(memory, cost, |memory| {
        memory
            .map_user_pages(&mut space, SCRATCH, PAGES, true)
            .unwrap();
        assert_eq!(memory.free_frames(), 0);
        for index in 0..PAGES {
            let mapping = space
                .inner
                .lookup(&memory.physical, SCRATCH + index * PAGE_SIZE)
                .unwrap();
            assert!(mapping.user && mapping.writable && !mapping.executable);
        }
        memory.unmap_user_pages(&mut space, SCRATCH, PAGES).unwrap();
    });
    memory.destroy_user(&mut space).unwrap();
    assert_eq!(memory.free_frames(), before);
}
