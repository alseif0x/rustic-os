// SPDX-License-Identifier: Apache-2.0
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Endpoint {
    pub(super) channel: u64,
    pub(super) side: usize,
}
pub(super) type Table = crate::handles::Table<Endpoint, 16, 8>;
