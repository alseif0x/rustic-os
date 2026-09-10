// SPDX-License-Identifier: Apache-2.0
//! Caller-owned waiting policy. The transport does not import console or service policy.
use crate::{Error, runtime};
pub trait Progress {
    fn wait(&mut self, endpoint: u64) -> Result<(), Error>;
}
pub struct Blocking;
impl Progress for Blocking {
    fn wait(&mut self, endpoint: u64) -> Result<(), Error> {
        runtime::wait_set(&[endpoint], 100).map_err(|_| Error::Protocol)
    }
}
