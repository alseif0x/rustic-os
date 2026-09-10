// SPDX-License-Identifier: Apache-2.0
use super::Device;
use crate::arch::interrupts::ticks;
use core::sync::atomic::{Ordering, fence};
use rustic_kernel::block::{Error, queue::Layout};
impl Device {
    /// One bounded observation; the caller can schedule another process between polls.
    pub(crate) fn poll(&mut self, data: &mut [u8]) -> Option<Result<(), Error>> {
        self.poll_request(data, 0)
    }
    /// Broker request identity is diagnostic only; queue ownership is unchanged.
    pub(crate) fn poll_request(
        &mut self,
        data: &mut [u8],
        _request: u64,
    ) -> Option<Result<(), Error>> {
        let pending = self.pending.as_mut()?;
        assert!(data.len() >= pending.length);
        let device_status = self.transport.read8(18);
        if device_status != 7 {
            #[cfg(feature = "sdk-test")]
            super::diagnostics::Completion {
                request: _request,
                kind: pending.kind,
                started: pending.started,
                now: ticks(),
                polls: pending.polls,
                expected: self.index,
                observed: self.dma.read(self.layout.used + 2, 2) as u16,
                device_status,
                descriptor: None,
                status: None,
            }
            .emit(Error::Protocol);
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
            #[cfg(feature = "sdk-test")]
            if let Err(error) = result {
                super::diagnostics::Completion {
                    request: _request,
                    kind: pending.kind,
                    started: pending.started,
                    now: ticks(),
                    polls: pending.polls,
                    expected: self.index,
                    observed,
                    device_status,
                    descriptor: Some(id),
                    status: Some(status),
                }
                .emit(error);
            }
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
        let now = ticks();
        if now.saturating_sub(pending.started) >= 25 || pending.polls >= 5_000_000 {
            #[cfg(feature = "sdk-test")]
            super::diagnostics::Completion {
                request: _request,
                kind: pending.kind,
                started: pending.started,
                now,
                polls: pending.polls,
                expected: self.index,
                observed,
                device_status,
                descriptor: None,
                status: None,
            }
            .emit(Error::Timeout);
            self.failed = true; // Submitted DMA remains owned until confirmed reset.
            self.pending = None;
            return Some(Err(Error::Timeout));
        }
        None
    }
}
