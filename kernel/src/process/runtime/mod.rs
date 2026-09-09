// SPDX-License-Identifier: Apache-2.0
//! Guest process composition. Validation/policy stay in the no-unsafe library.
mod error;
mod loader;
mod manager;
mod tests;
use error::Error;
pub(crate) use tests::verify;
