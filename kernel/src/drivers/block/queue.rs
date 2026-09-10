// SPDX-License-Identifier: Apache-2.0
use super::device::Device;
use crate::arch::interrupts::ticks;
use core::sync::atomic::{Ordering, fence};
use rustic_kernel::block::Error;
impl Device {
    fn descriptor(&mut self, id: usize, address: u64, length: u32, flags: u16, next: u16) {
        let offset = id * 16;
        self.dma.write(offset, 8, address);
        self.dma.write(offset + 8, 4, u64::from(length));
        self.dma.write(offset + 12, 2, u64::from(flags));
        self.dma.write(offset + 14, 2, u64::from(next));
    }
    pub(crate) fn start(
        &mut self,
        kind: u32,
        sector: u64,
        data: &[u8],
        notify: bool,
    ) -> Result<(), Error> {
        assert!(data.len() <= 512);
        if self.failed || self.transport.read8(18) != 7 {
            return Err(Error::Protocol);
        }
        if self.pending.is_some() {
            return Err(Error::Busy);
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
        self.pending = Some(super::device::Pending {
            kind,
            length: data.len(),
            budget: rustic_kernel::block::deadline::RequestBudget::new(ticks()),
        });
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
        Ok(())
    }
}
