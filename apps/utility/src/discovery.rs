// SPDX-License-Identifier: Apache-2.0
//! A deterministic client asks the same question the shell asks, under its own grant.
use rustic_sdk::{abi::services::Method, files::Client};

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
