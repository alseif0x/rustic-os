// SPDX-License-Identifier: Apache-2.0
//! Owned ring-0 GDT/TSS and emergency double-fault stack. No userspace segments yet.
use core::mem::size_of;

#[repr(C, packed)]
struct Tss {
    reserved0: u32,
    rsp: [u64; 3],
    reserved1: u64,
    ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    iomap: u16,
}

const _: () = assert!(size_of::<Tss>() == 104);

#[repr(C, align(16))]
struct Stack([u8; 16 * 1024]);

static mut STACK: Stack = Stack([0; 16 * 1024]);
static mut TSS: Tss = Tss {
    reserved0: 0,
    rsp: [0; 3],
    reserved1: 0,
    ist: [0; 7],
    reserved2: 0,
    reserved3: 0,
    iomap: size_of::<Tss>() as u16,
};
static mut GDT: [u64; 5] = [0, 0x00af_9a00_0000_ffff, 0x00cf_9200_0000_ffff, 0, 0];

#[repr(C, packed)]
pub(super) struct Descriptor {
    pub(super) limit: u16,
    pub(super) base: u64,
}

pub(super) fn emergency_contains(address: usize) -> bool {
    let start = core::ptr::addr_of!(STACK) as usize;
    (start..start + size_of::<Stack>()).contains(&address)
}

/// Called exactly once, by initialization with IF=0, before any IDT uses this TSS.
pub(super) unsafe fn initialize() {
    let stack_top = core::ptr::addr_of!(STACK) as u64 + size_of::<Stack>() as u64;
    let base = core::ptr::addr_of!(TSS) as u64;
    let limit = (size_of::<Tss>() - 1) as u64;
    // SAFETY: Unique bootstrap owner; no references to packed/static mutable fields
    // escape. Tables and emergency stack remain mapped for the kernel lifetime.
    unsafe {
        core::ptr::addr_of_mut!(TSS.ist).write_unaligned([stack_top, 0, 0, 0, 0, 0, 0]);
        core::ptr::addr_of_mut!(GDT[3]).write(
            limit | ((base & 0xff_ffff) << 16) | (0x89 << 40) | ((base & 0xff00_0000) << 32),
        );
        core::ptr::addr_of_mut!(GDT[4]).write(base >> 32);
        let descriptor = Descriptor {
            limit: (size_of::<[u64; 5]>() - 1) as u16,
            base: core::ptr::addr_of!(GDT) as u64,
        };
        core::arch::asm!(
            "lgdt [{table}]", "push 0x08", "lea rax, [rip + 2f]", "push rax", "retfq",
            "2:", "mov ax, 0x10", "mov ds, ax", "mov es, ax", "mov ss, ax",
            "mov ax, 0x18", "ltr ax", table = in(reg) &descriptor, out("rax") _,
        );
    }
}
