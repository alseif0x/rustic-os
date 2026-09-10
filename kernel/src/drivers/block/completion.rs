// SPDX-License-Identifier: Apache-2.0
use super::Device;
use crate::arch::interrupts::ticks;
use core::sync::atomic::{Ordering, fence};
use rustic_kernel::block::{Error, queue::Layout};
impl Device {
    /// One bounded observation; the caller can schedule another process between polls.
    pub(crate) fn poll(&mut self, data: &mut [u8]) -> Option<Result<(), Error>> {
        let pending = self.pending.as_mut()?;
        assert!(data.len() >= pending.length);
        if self.transport.read8(18) != 7 {
            self.failed = true;
            self.pending = None;
            return Some(Err(Error::Protocol));
        }
        let base = self.layout.pages * 4096;
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
            } else {
                self.index = observed;
                if result.is_ok() && pending.kind == 0 {
                    for (i, value) in data[..pending.length].iter_mut().enumerate() {
                        *value = self.dma.read(base + 16 + i, 1) as u8;
                    }
                }
            }
            self.pending = None;
            return Some(result);
        }
        pending.polls += 1;
        if ticks().saturating_sub(pending.started) >= 25 || pending.polls >= 5_000_000 {
            self.failed = true; // Submitted DMA remains owned until confirmed reset.
            self.pending = None;
            return Some(Err(Error::Timeout));
        }
        None
    }
}
