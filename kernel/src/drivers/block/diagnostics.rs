// SPDX-License-Identifier: Apache-2.0
//! Failure-only native evidence. No payload, DMA address or grant token is logged.
use crate::arch::Serial;
use core::fmt::Write;
use rustic_abi::block::Operation;
use rustic_kernel::block::Error;

/// Observations already made by the sole queue owner, before it drops Pending.
pub(super) struct Completion {
    pub(super) request: u64,
    pub(super) kind: u32,
    pub(super) started: u64,
    pub(super) now: u64,
    pub(super) polls: u64,
    pub(super) stalled_polls: u64,
    pub(super) expected: u16,
    pub(super) observed: u16,
    pub(super) device_status: u8,
    pub(super) descriptor: Option<u32>,
    pub(super) status: Option<u8>,
}

impl Completion {
    pub(super) fn emit(self, error: Error) {
        if let Some(mut serial) = Serial::take() {
            // Request zero identifies direct kernel fixtures, which have no Broker ID.
            // Absent descriptor/status means no used entry was consumed, not status 0.
            let _ = writeln!(
                serial,
                "RUSTIC BLOCK_FAILURE phase=completion request={} kind={} reason={error:?} started={} now={} elapsed_ticks={} polls={} stalled_polls={} expected={} observed={} device_status={} descriptor={:?} status={:?}",
                self.request,
                self.kind,
                self.started,
                self.now,
                self.now.saturating_sub(self.started),
                self.polls,
                self.stalled_polls,
                self.expected,
                self.observed,
                self.device_status,
                self.descriptor,
                self.status,
            );
        }
    }
}

pub(crate) fn start_failure(
    request: u64,
    owner: u64,
    operation: Operation,
    sector: u64,
    now: u64,
    error: Error,
) {
    if let Some(mut serial) = Serial::take() {
        let _ = writeln!(
            serial,
            "RUSTIC BLOCK_FAILURE phase=start request={request} owner={owner} operation={operation:?} sector={sector} now={now} reason={error:?}",
        );
    }
}
