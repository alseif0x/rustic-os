// SPDX-License-Identifier: Apache-2.0
//! Validated boot data. Protocol-specific decoding belongs in a separate adapter.

mod region;

pub use region::{MemoryRegion, RegionError};
