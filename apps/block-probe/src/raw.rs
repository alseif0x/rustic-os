// SPDX-License-Identifier: Apache-2.0
//! Deliberately malformed syscall arguments, confined to the adversarial guest probe.
use rustic_sdk::abi::block::{Completion, RESULT_BYTES};
#[repr(align(4096))]
struct Pages([u8; 8192]);
static mut PAGES: Pages = Pages([0; 8192]);
pub(super) fn cross_page() -> u64 {
    // SAFETY: Only take the address of private process storage; no reference is formed.
    unsafe { (&raw mut PAGES.0).cast::<u8>() as u64 + 4096 - 272 }
}
pub(super) fn cross_result() -> Completion {
    // SAFETY: This process's private static allocation contains the entire range;
    // the caller has completed synchronous RESULT copy-out and no other user thread exists.
    let bytes = unsafe { core::slice::from_raw_parts(cross_page() as *const u8, RESULT_BYTES) };
    Completion::decode(bytes).unwrap()
}
pub(super) fn call(number: u64, a: u64, b: u64, c: u64) -> u64 {
    let mut value = number;
    // SAFETY: Guest trap preserves GPRs except RAX. Integer pointer arguments are
    // deliberately invalid in rejection tests; Rust never dereferences them here.
    // Real input/output buffers are private and alive through the synchronous call.
    unsafe {
        core::arch::asm!("int 0x80", inout("rax") value, in("rdi") a, in("rsi") b, in("rdx") c);
    }
    value
}
