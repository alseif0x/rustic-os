// SPDX-License-Identifier: Apache-2.0
//! Trusted admission, before allocating process resources. No capability grants.
use super::{Error as ProcessError, manager::Manager};
use crate::arch::memory::Memory;
use rustic_abi::application::Manifest;
use rustic_kernel::process::lifecycle::Pid;
#[derive(Debug)]
pub(super) enum Error {
    Manifest(rustic_abi::application::Error),
    Executable,
    Denied,
    Process(ProcessError),
}
pub(super) fn launch(
    manager: &mut Manager,
    memory: &mut Memory,
    manifest: &[u8],
    name: &str,
    elf: &[u8],
    available: u64,
) -> Result<Pid, Error> {
    let manifest = Manifest::parse(manifest).map_err(Error::Manifest)?;
    if manifest.executable != name {
        return Err(Error::Executable);
    }
    if !manifest.admitted(available) {
        return Err(Error::Denied);
    }
    manager.create(memory, elf, [0; 3]).map_err(Error::Process)
}
