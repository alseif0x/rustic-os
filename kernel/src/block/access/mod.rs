// SPDX-License-Identifier: Apache-2.0
//! Pure admission, bounded queue and completion ownership; no device or user pointers.
mod grants;
mod queue;
pub use grants::Grant;
pub use queue::{Broker, Pending};
use rustic_abi::block::Error;
impl From<crate::handles::Error> for Error {
    fn from(value: crate::handles::Error) -> Self {
        match value {
            crate::handles::Error::Handle => Self::Handle,
            crate::handles::Error::Denied => Self::Denied,
            crate::handles::Error::Quota => Self::Quota,
        }
    }
}
