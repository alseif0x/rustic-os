// SPDX-License-Identifier: Apache-2.0
//! A deterministic client asks the same question the shell asks, under its own grant.
use rustic_sdk::{abi::services::Method, files::Client};

pub(super) fn profile(files: &mut Client, cancel: bool) -> [u64; 8] {
    let method = if cancel {
        Method::OperationsCancel
    } else {
        Method::OperationsGet
    };
    match files.negotiate_lifecycle(method) {
        Ok(selected) => {
            let d = selected.descriptor();
            let packed = u64::from(d.availability as u8)
                | (u64::from(d.limits.retained_operations) << 8)
                | (u64::from(d.limits.execution_tickets) << 16)
                | (u64::from(d.limits.active_publications) << 24);
            // Success already requires the full exact digest in the SDK decoder.
            [
                0,
                packed,
                selected.responder(),
                1,
                rustic_sdk::abi::files::negotiation::VERSION,
                0,
                0,
                0,
            ]
        }
        Err(e) => [e as u64, 0, 0, 0, 0, 0, 0, 0],
    }
}

/// Packs one availability byte per catalog method so the owner can compare this
/// answer with the manual one. Rights differ; the reported facts must not.
pub(super) fn report(files: &mut Client) -> [u64; 8] {
    match files.capabilities() {
        Ok(report) => {
            let mut packed = 0;
            for (index, method) in Method::ALL.into_iter().enumerate() {
                packed |= u64::from(report.of(method) as u8) << (index * 8);
            }
            [
                0,
                packed,
                u64::from(report.bounds.max_inline_bytes),
                u64::from(report.bounds.max_page_items),
                u64::from(report.bounds.receipt_capacity),
                0,
                0,
                0,
            ]
        }
        Err(error) => [error as u64, 0, 0, 0, 0, 0, 0, 0],
    }
}
