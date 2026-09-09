// SPDX-License-Identifier: Apache-2.0
//! Scoped single-CPU transition. The IRQ handler never owns a scheduler/allocator.
use super::frame::Frame;

pub(crate) struct Event {
    pub(crate) vector: u64,
    pub(crate) error: u64,
    pub(crate) address: u64,
}
struct Exchange {
    user: Frame,
    kernel: Frame,
    entered: bool,
    address: u64,
}
static mut ACTIVE: *mut Exchange = core::ptr::null_mut();

unsafe extern "C" {
    fn rustic_user_roundtrip();
}

/// Caller keeps IF=0 and a live user root active, with common upper kernel/stack
/// mappings. It must restore its kernel root after this synchronous return.
pub(crate) unsafe fn run(frame: &mut Frame) -> Event {
    assert!(frame.valid_user());
    frame.sanitize();
    let mut exchange = Exchange {
        user: *frame,
        kernel: Frame::default(),
        entered: false,
        address: 0,
    };
    let old_cr0: u64;
    // SAFETY: One CPU, IF=0, no nested run. Exchange lives on a common kernel
    // stack until return; its unique raw pointer is accessed only in the handler
    // while this Rust activation is suspended. No reference survives the call.
    unsafe {
        assert!(ACTIVE.is_null());
        ACTIVE = &raw mut exchange;
        core::arch::asm!("mov {}, cr0", out(reg) old_cr0, options(nomem, nostack));
        // No FP/SIMD context ABI in R0: TS traps x87/MMX/SSE before use.
        core::arch::asm!("mov cr0, {}", in(reg) old_cr0 | 8, options(nostack));
        rustic_user_roundtrip();
        core::arch::asm!("mov cr0, {}", in(reg) old_cr0, options(nostack));
        ACTIVE = core::ptr::null_mut();
    }
    *frame = exchange.user;
    Event {
        vector: frame.vector,
        error: frame.error,
        address: exchange.address,
    }
}

pub(super) fn dispatch(frame: &mut Frame) -> bool {
    // NMI/#DF/machine check remain kernel-terminal, including during user entry.
    if matches!(frame.vector, 2 | 8 | 18) {
        return false;
    }
    // SAFETY: Interrupt gates have IF=0. Only run installs a live pointer; run is
    // suspended while this exclusive borrow exists. NMI does not borrow ACTIVE.
    unsafe {
        if ACTIVE.is_null() {
            return false;
        }
        let exchange = &mut *ACTIVE;
        if frame.vector == 129 && frame.cs == 8 && !exchange.entered {
            exchange.kernel = *frame;
            exchange.entered = true;
            *frame = exchange.user;
            return true;
        }
        if frame.cs & 3 != 3 || !exchange.entered {
            return false;
        }
        exchange.user = *frame;
        core::arch::asm!("mov {}, cr2", out(reg) exchange.address, options(nomem, nostack, preserves_flags));
        *frame = exchange.kernel;
        true
    }
}
