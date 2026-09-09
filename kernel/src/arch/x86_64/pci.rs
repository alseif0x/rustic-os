// SPDX-License-Identifier: Apache-2.0
//! One explicitly assigned R0 device, PCI 00:06.0. No bus-wide probing or BAR relocation.
use super::io;
use core::sync::atomic::{AtomicBool, Ordering};
use rustic_kernel::block::Error;
static CLAIMED: AtomicBool = AtomicBool::new(false);
pub(crate) struct BlockTransport {
    port: u16,
}
fn config(offset: u16) -> u32 {
    // SAFETY: Single CPU; IRQ handlers never access PCI config ports. Fixed BDF,
    // aligned config dword, no concurrent selector writer.
    unsafe {
        io::write_u32(0xcf8, 0x8000_3000 | u32::from(offset & !3));
        io::read_u32(0xcfc)
    }
}
fn command(value: u16) {
    // SAFETY: Same exclusive PCI selector invariant; 16-bit command write avoids
    // touching adjacent write-one-to-clear PCI status bits.
    unsafe {
        io::write_u32(0xcf8, 0x8000_3004);
        io::write_u16(0xcfc, value);
    }
}
impl BlockTransport {
    pub(crate) fn claim() -> Result<Self, Error> {
        if CLAIMED.swap(true, Ordering::Relaxed) {
            return Err(Error::Busy);
        }
        let id = config(0);
        let bar = config(0x10);
        if id != 0x1001_1af4
            || config(8) & 255 != 0
            || bar & 1 == 0
            || bar & !3 == 0
            || bar & !3 > 0xffc0
        {
            CLAIMED.store(false, Ordering::Relaxed);
            return Err(if id == u32::MAX {
                Error::Missing
            } else {
                Error::Unsupported
            });
        }
        // I/O decode, bus master and INTx disable. MSI-X stays disabled by R0 vectors=0.
        command((config(4) as u16 | 0x405) & !2);
        Ok(Self {
            port: (bar & !3) as u16,
        })
    }
    pub(crate) fn read8(&self, offset: u16) -> u8 {
        assert!(offset < 64);
        // SAFETY: Claimed I/O BAR with checked bounded offset, ring 0 only.
        unsafe { io::read_u8(self.port + offset) }
    }
    pub(crate) fn read16(&self, offset: u16) -> u16 {
        assert!(offset < 64 && offset.is_multiple_of(2));
        // SAFETY: Claimed aligned 16-bit register in the bounded I/O BAR.
        unsafe { io::read_u16(self.port + offset) }
    }
    pub(crate) fn read32(&self, offset: u16) -> u32 {
        assert!(offset < 64 && offset.is_multiple_of(4));
        // SAFETY: Claimed aligned 32-bit register in the bounded I/O BAR.
        unsafe { io::read_u32(self.port + offset) }
    }
    pub(crate) fn write8(&mut self, offset: u16, value: u8) {
        assert!(offset < 64);
        // SAFETY: Unique claimed I/O BAR; bounded byte register.
        unsafe {
            io::write_u8(self.port + offset, value);
        }
    }
    pub(crate) fn write16(&mut self, offset: u16, value: u16) {
        assert!(offset < 64 && offset.is_multiple_of(2));
        // SAFETY: Unique claimed I/O BAR; bounded aligned register.
        unsafe {
            io::write_u16(self.port + offset, value);
        }
    }
    pub(crate) fn write32(&mut self, offset: u16, value: u32) {
        assert!(offset < 64 && offset.is_multiple_of(4));
        // SAFETY: Unique claimed I/O BAR; bounded aligned register.
        unsafe {
            io::write_u32(self.port + offset, value);
        }
    }
    pub(crate) fn reset(&mut self) -> Result<(), Error> {
        self.write8(18, 0);
        for _ in 0..100_000 {
            if self.read8(18) == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        command(config(4) as u16 & !4); // Additional containment; not proof that DMA drained.
        Err(Error::Reset)
    }
    pub(crate) fn release(self) {
        command(config(4) as u16 & !4);
        CLAIMED.store(false, Ordering::Relaxed);
    }
}
