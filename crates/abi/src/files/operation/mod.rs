// SPDX-License-Identifier: Apache-2.0
//! Completed-operation profile. Identifiers confer no authority; staging is volatile.
mod identity;
mod receipt;
mod request;
pub use identity::{Instance, Key, OperationId};
pub use receipt::{Operation, RECEIPT_BYTES};
pub use request::{Lookup, Replacement, Retry};
