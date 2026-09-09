// SPDX-License-Identifier: Apache-2.0
//! R0 dual 8259. Only PIT IRQ0 is unmasked; all controller access has IF=0.
use super::super::io;

fn write(port: u16, value: u8) {
    // SAFETY: This module exclusively owns the PIC ports on the single R0 CPU.
    unsafe { io::write_u8(port, value) };
}

fn read(port: u16) -> u8 {
    // SAFETY: PIC ownership as above; no interrupt can preempt these transactions.
    unsafe { io::read_u8(port) }
}

pub(super) fn initialize() {
    write(0x21, 0xff);
    write(0xa1, 0xff);
    write(0x20, 0x11);
    write(0xa0, 0x11);
    write(0x21, 32);
    write(0xa1, 40);
    write(0x21, 4);
    write(0xa1, 2);
    write(0x21, 1);
    write(0xa1, 1);
    write(0x21, 0xfe);
    write(0xa1, 0xff);
}

pub(super) fn mask_timer_for_test() {
    write(0x21, 0xff);
}

/// False for spurious IRQ7/15. A spurious slave IRQ still requires master EOI.
pub(super) fn acknowledge(vector: u64) -> bool {
    if vector == 39 || vector == 47 {
        let port = if vector == 39 { 0x20 } else { 0xa0 };
        write(port, 0x0b);
        if read(port) & 0x80 == 0 {
            if vector == 47 {
                write(0x20, 0x20);
            }
            return false;
        }
    }
    if vector >= 40 {
        write(0xa0, 0x20);
    }
    write(0x20, 0x20);
    true
}
