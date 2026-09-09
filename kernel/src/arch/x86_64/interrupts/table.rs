// SPDX-License-Identifier: Apache-2.0
use super::segments::Descriptor;
use core::mem::size_of;

#[derive(Clone, Copy)]
#[repr(C)]
struct Gate {
    low: u16,
    selector: u16,
    ist: u8,
    flags: u8,
    middle: u16,
    high: u32,
    reserved: u32,
}

const _: () = assert!(size_of::<Gate>() == 16);

impl Gate {
    const EMPTY: Self = Self {
        low: 0,
        selector: 0,
        ist: 0,
        flags: 0,
        middle: 0,
        high: 0,
        reserved: 0,
    };

    fn new(address: u64, ist: u8) -> Self {
        Self {
            low: address as u16,
            selector: 8,
            ist,
            flags: 0x8e,
            middle: (address >> 16) as u16,
            high: (address >> 32) as u32,
            reserved: 0,
        }
    }
}

static mut IDT: [Gate; 256] = [Gate::EMPTY; 256];

unsafe extern "C" {
    static rustic_isr_table: [u64; 48];
    fn rustic_isr_default();
}

/// Unique bootstrap caller, IF=0, owned GDT/TSS already loaded.
pub(super) unsafe fn initialize() {
    // SAFETY: No live references or interrupt readers while initializing. Assembly
    // provides exactly 48 addresses; all 256 gates target resident ring-0 code.
    unsafe {
        for vector in 0..256 {
            let address = if vector < 48 {
                rustic_isr_table[vector]
            } else {
                rustic_isr_default as *const () as u64
            };
            core::ptr::addr_of_mut!(IDT[vector]).write(Gate::new(address, u8::from(vector == 8)));
        }
        let descriptor = Descriptor {
            limit: (size_of::<[Gate; 256]>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as u64,
        };
        core::arch::asm!("lidt [{}]", in(reg) &descriptor, options(readonly, nostack));
    }
}

/// Deliberate terminal fixture: a fault during #GP delivery escalates to #DF.
pub(super) unsafe fn invalidate_gp() {
    // SAFETY: Fixture has IF=0 and never resumes normal execution; no references alias.
    unsafe { core::ptr::addr_of_mut!(IDT[13].flags).write(0) };
}
