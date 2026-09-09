// SPDX-License-Identifier: Apache-2.0
//! Pure physical ownership and page policy; no CPU instructions or pointer access.
mod frames;
mod page;

pub use frames::{FrameAllocator, FrameError};
pub use page::{PAGE_SIZE, PagePermissions, VirtualPage, canonical};
