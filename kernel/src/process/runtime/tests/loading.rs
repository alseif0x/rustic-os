// SPDX-License-Identifier: Apache-2.0
use super::*;
use rustic_kernel::process::{elf, lifecycle};

pub(super) fn verify(manager: &mut Manager, memory: &mut Memory) -> usize {
    let before = memory.free_frames();
    for _ in 0..16 {
        assert!(matches!(
            manager.create(memory, &[0; 64], [0; 3]),
            Err(Error::Elf(_))
        ));
        assert_eq!(memory.free_frames(), before);
    }
    // A valid first segment followed by a bad entry is rejected before allocation.
    let mut header = [0u8; 8200];
    header.copy_from_slice(image(false));
    header[24..32].copy_from_slice(&elf::STACK_TOP.to_le_bytes());
    assert!(matches!(
        manager.create(memory, &header, [0; 3]),
        Err(Error::Elf(elf::Error::Entry))
    ));
    assert_eq!(memory.free_frames(), before);
    for remaining in [0, 5, 9] {
        memory.verify_user_oom(remaining, |memory| {
            assert_eq!(
                manager.create(memory, image(false), [0; 3]),
                Err(Error::Memory(crate::arch::memory::Error::Frames(
                    rustic_kernel::memory::FrameError::Exhausted
                )))
            );
        });
    }
    let mut pids = [Pid(0); lifecycle::CAPACITY];
    for pid in &mut pids {
        *pid = manager.create(memory, image(false), [0; 3]).unwrap();
    }
    let full = memory.free_frames();
    assert_eq!(
        manager.create(memory, image(false), [0; 3]),
        Err(Error::Process(lifecycle::Error::Full))
    );
    assert_eq!(memory.free_frames(), full);
    for pid in pids {
        manager.kill(pid).unwrap();
        assert_eq!(manager.wait(memory, pid).unwrap(), Some(Exit::Killed));
    }
    assert_eq!(memory.free_frames(), before);
    before - full
}
