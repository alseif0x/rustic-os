// SPDX-License-Identifier: Apache-2.0
//! Owner-bound typed token domains shared by kernel resource brokers.
mod table;
pub(crate) use table::Table;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Handle,
    Denied,
    Quota,
}
