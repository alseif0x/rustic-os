// SPDX-License-Identifier: Apache-2.0
use super::super::io;

pub(super) const INPUT_HZ: u32 = 1_193_182;
pub(super) const DIVISOR: u16 = 11_932;

pub(super) fn initialize() {
    // SAFETY: IF=0; exclusive channel 0 ownership. Mode 2, binary, low/high bytes.
    unsafe {
        io::write_u8(0x43, 0x34);
        io::write_u8(0x40, DIVISOR as u8);
        io::write_u8(0x40, (DIVISOR >> 8) as u8);
    }
}
