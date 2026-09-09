// SPDX-License-Identifier: Apache-2.0

/// Caller must own the port and execute at sufficient privilege.
pub(super) unsafe fn write_u8(port: u16, value: u8) {
    // SAFETY: The caller guarantees privilege and ownership of the device port.
    unsafe {
        core::arch::asm!("out dx, al", in("dx") port, in("al") value,
            options(nostack, preserves_flags));
    }
}

/// Caller must own the port and execute at sufficient privilege.
pub(super) unsafe fn read_u8(port: u16) -> u8 {
    let value;
    // SAFETY: The caller guarantees privilege and ownership of the device port.
    unsafe {
        core::arch::asm!("in al, dx", in("dx") port, out("al") value,
            options(nostack, preserves_flags));
    }
    value
}

/// Caller must own the port and execute at sufficient privilege.
pub(super) unsafe fn write_u32(port: u16, value: u32) {
    // SAFETY: The caller guarantees privilege and ownership of the device port.
    unsafe {
        core::arch::asm!("out dx, eax", in("dx") port, in("eax") value,
            options(nostack, preserves_flags));
    }
}

/// Caller owns the port and executes at ring 0.
pub(super) unsafe fn read_u16(port: u16) -> u16 {
    let value;
    // SAFETY: Caller guarantees port ownership and privilege.
    unsafe {
        core::arch::asm!("in ax, dx", in("dx") port, out("ax") value, options(nostack, preserves_flags));
    }
    value
}
/// Caller owns the port and executes at ring 0.
pub(super) unsafe fn read_u32(port: u16) -> u32 {
    let value;
    // SAFETY: Caller guarantees port ownership and privilege.
    unsafe {
        core::arch::asm!("in eax, dx", in("dx") port, out("eax") value, options(nostack, preserves_flags));
    }
    value
}
/// Caller owns the port and executes at ring 0.
pub(super) unsafe fn write_u16(port: u16, value: u16) {
    // SAFETY: Caller guarantees port ownership and privilege.
    unsafe {
        core::arch::asm!("out dx, ax", in("dx") port, in("ax") value, options(nostack, preserves_flags));
    }
}
