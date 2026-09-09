// SPDX-License-Identifier: Apache-2.0
//! The sole instruction boundary; unavailable on host operating systems.
/// Caller guarantees pointer arguments remain valid for this synchronous call.
pub(crate) unsafe fn call(number: u64, a: u64, b: u64, c: u64) -> u64 {
    let mut result = number;
    // SAFETY: Guest INT 0x80 preserves all general registers except RAX.
    // Memory may be read/written by the kernel: no nomem/readonly promises.
    // The entry stack is private; no borrowed buffer survives the return.
    unsafe {
        core::arch::asm!("int 0x80",
        inout("rax") result, in("rdi") a, in("rsi") b, in("rdx") c);
    }
    result
}
