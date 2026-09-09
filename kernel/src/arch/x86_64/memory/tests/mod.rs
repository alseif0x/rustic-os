// SPDX-License-Identifier: Apache-2.0
mod allocation;
pub(super) use allocation::with_free_frames;
mod faults;
mod spaces;
mod splitting;

use super::{Memory, bootstrap};
use crate::arch::Serial;
use core::fmt::Write;
use rustic_kernel::memory::VirtualPage;

pub(super) use faults::fault;
const SCRATCH: u64 = 0x4000_0000;
fn page() -> VirtualPage {
    VirtualPage::new(SCRATCH).unwrap()
}

pub(super) fn verify(memory: &mut Memory) {
    let before = memory.physical.frames.free_count();
    let code = memory
        .kernel
        .lookup(&memory.physical, bootstrap::text_start())
        .unwrap();
    assert!(!code.writable && code.executable && !code.user);
    let alias = memory
        .kernel
        .lookup(&memory.physical, memory.layout.hhdm + code.physical)
        .unwrap();
    assert!(!alias.writable && !alias.executable && !alias.user);
    let rodata = memory
        .kernel
        .lookup(&memory.physical, bootstrap::rodata_start())
        .unwrap();
    assert!(!rodata.writable && !rodata.executable && !rodata.user);
    assert!(
        memory
            .kernel
            .lookup(&memory.physical, crate::arch::interrupts::emergency_guard())
            .is_none()
    );
    allocation::pages(memory);
    splitting::verify(memory);
    spaces::verify(memory);
    let exhausted = allocation::exhaust(memory);
    assert_eq!(memory.physical.frames.free_count(), before);
    if let Some(mut serial) = Serial::take() {
        let total = memory.physical.frames.total_count();
        let _ = writeln!(
            serial,
            "RUSTIC MEMORY verified=1 page_bytes=4096 metadata_bytes=65536 managed_frames={} table_frames={} free_before={} free_after={} exhausted={} rollback=1 zero_reuse=1 spaces=2 wx=1 aliases=1 guard=1",
            total,
            total - before,
            before,
            memory.physical.frames.free_count(),
            exhausted
        );
        serial.flush();
    }
}

fn read(address: u64) -> u64 {
    let value: u64;
    // SAFETY: Fixture supplies a live mapped page and keeps its space active. Raw
    // asm deliberately avoids creating Rust references across address-space switches.
    unsafe {
        core::arch::asm!("mov {}, qword ptr [{}]", out(reg) value, in(reg) address, options(nostack, readonly, preserves_flags));
    }
    value
}

fn write(address: u64, value: u64) {
    // SAFETY: Fixture owns a mapped writable page; no Rust references alias its data.
    unsafe {
        core::arch::asm!("mov qword ptr [{}], {}", in(reg) address, in(reg) value, options(nostack, preserves_flags));
    }
}
