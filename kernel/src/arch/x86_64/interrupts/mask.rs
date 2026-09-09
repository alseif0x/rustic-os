// SPDX-License-Identifier: Apache-2.0
use core::marker::PhantomData;

/// CPU-local guard. Nesting preserves IF; it must never move to another CPU.
pub(super) struct Mask {
    enabled: bool,
    _local: PhantomData<*mut ()>,
}

impl Mask {
    pub(super) fn acquire() -> Self {
        let flags: u64;
        // SAFETY: Ring 0, one R0 CPU. Stack is valid; CLI fences compiler memory operations.
        unsafe { core::arch::asm!("pushfq", "pop {}", "cli", out(reg) flags) };
        Self {
            enabled: flags & (1 << 9) != 0,
            _local: PhantomData,
        }
    }

    pub(super) fn was_enabled(&self) -> bool {
        self.enabled
    }

    pub(super) fn sleep(&self) {
        // SAFETY: Called with IF=0 after checking the deadline. STI's interrupt shadow
        // covers HLT, so a pending interrupt cannot be lost between check and sleep.
        // CLI restores the guard invariant before returning to Rust.
        unsafe { core::arch::asm!("sti", "hlt", "cli", options(nostack)) };
    }
}

impl Drop for Mask {
    fn drop(&mut self) {
        if self.enabled {
            // SAFETY: Restore only this CPU's previous IF, with its installed IDT.
            unsafe { core::arch::asm!("sti", options(nostack)) };
        }
    }
}
