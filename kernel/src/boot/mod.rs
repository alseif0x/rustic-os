// SPDX-License-Identifier: Apache-2.0
//! Validated boot data. Protocol-specific decoding belongs in a separate adapter.

mod map;
mod mode;
mod region;

pub use map::{MapError, MapSummary, validate_map};
pub use mode::BootMode;
pub use region::{MemoryRegion, RegionError};
