// SPDX-License-Identifier: Apache-2.0
//! Trusted bootstrap; grants require a current live process, never manifest metadata.
use super::manager::Manager;
use rustic_abi::block::Error;
use rustic_kernel::{block::access::Grant, process::lifecycle::Pid};
impl Manager {
    pub(super) fn grant_block(&mut self, pid: Pid, grant: Grant) -> Result<u64, Error> {
        self.live(pid).map_err(|_| Error::Handle)?;
        self.block
            .broker
            .grant(pid.0, grant, self.block.geometry()?.sectors)
    }
}
