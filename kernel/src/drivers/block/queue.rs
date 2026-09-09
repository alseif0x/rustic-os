// SPDX-License-Identifier: Apache-2.0
use super::device::Device;
use crate::arch::interrupts::ticks;
use core::sync::atomic::{Ordering, fence};
use rustic_kernel::block::{Error, queue::Layout};
impl Device {
    fn descriptor(&mut self, id: usize, address: u64, length: u32, flags: u16, next: u16) {
        let offset = id * 16;
        self.dma.write(offset, 8, address);
        self.dma.write(offset + 8, 4, u64::from(length));
        self.dma.write(offset + 12, 2, u64::from(flags));
        self.dma.write(offset + 14, 2, u64::from(next));
    }
    pub(super) fn submit(
        &mut self,
        kind: u32,
        sector: u64,
        data: &mut [u8],
        notify: bool,
    ) -> Result<(), Error> {
        assert!(data.len() <= 512);
        if self.failed || self.transport.read8(18) != 7 {
            return Err(Error::Protocol);
        }
        let base = self.layout.pages * 4096;
        self.dma.write(base, 4, u64::from(kind));
        self.dma.write(base + 4, 4, 0);
        self.dma.write(base + 8, 8, sector);
        self.dma.write(base + 528, 1, 255);
        for (i, value) in data.iter().enumerate() {
            self.dma.write(
                base + 16 + i,
                1,
                if kind == 1 { u64::from(*value) } else { 0 },
            );
        }
        self.descriptor(
            0,
            self.dma.physical(base),
            16,
            1,
            if data.is_empty() { 2 } else { 1 },
        );
        self.descriptor(
            1,
            self.dma.physical(base + 16),
            data.len() as u32,
            if kind == 0 { 3 } else { 1 },
            2,
        );
        self.descriptor(2, self.dma.physical(base + 528), 1, 2, 0);
        self.dma.write(
            self.layout.available + 4 + usize::from(self.index) % self.layout.size * 2,
            2,
            0,
        );
        fence(Ordering::SeqCst);
        self.dma.write(
            self.layout.available + 2,
            2,
            u64::from(self.index.wrapping_add(1)),
        );
        fence(Ordering::SeqCst);
        if notify {
            self.transport.write16(16, 0);
        }
        let started = ticks();
        for _ in 0..5_000_000 {
            let observed = self.dma.read(self.layout.used + 2, 2) as u16;
            if observed != self.index {
                fence(Ordering::SeqCst);
                let id = self.dma.read(
                    self.layout.used + 4 + usize::from(self.index) % self.layout.size * 8,
                    4,
                ) as u32;
                let status = self.dma.read(base + 528, 1) as u8;
                let result = Layout::completed(self.index, observed, id, status);
                if result == Err(Error::Protocol) {
                    self.failed = true;
                    return result;
                }
                self.index = observed;
                if result.is_ok() && kind == 0 {
                    for (i, value) in data.iter_mut().enumerate() {
                        *value = self.dma.read(base + 16 + i, 1) as u8;
                    }
                }
                return result;
            }
            if ticks().saturating_sub(started) >= 25 {
                break;
            }
            core::hint::spin_loop();
        }
        self.failed = true; // Never overwrite/reuse buffers still offered to the device.
        Err(Error::Timeout)
    }
}
