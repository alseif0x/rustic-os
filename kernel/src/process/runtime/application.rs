// SPDX-License-Identifier: Apache-2.0
//! Trusted admission, before allocating process resources. No capability grants.
use super::{Error as ProcessError, manager::Manager};
use crate::arch::memory::Memory;
use rustic_abi::application::Manifest;
use rustic_kernel::process::lifecycle::Pid;
use sha2::{Digest, Sha256};
#[derive(Debug)]
pub(super) enum Error {
    Manifest(rustic_abi::application::Error),
    Executable,
    ArtifactDigest,
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
    if elf.len() > rustic_kernel::process::elf::MAX_BYTES {
        return Err(Error::Process(ProcessError::Elf(
            rustic_kernel::process::elf::Error::Header,
        )));
    }
    let artifact_sha256: [u8; 32] = Sha256::digest(elf).into();
    if artifact_sha256 != manifest.artifact_sha256 {
        return Err(Error::ArtifactDigest);
    }
    if !manifest.admitted(available) {
        return Err(Error::Denied);
    }
    rustic_kernel::process::elf::Image::parse(elf)
        .map_err(|error| Error::Process(ProcessError::Elf(error)))?;
    manager.create(memory, elf, [0; 3]).map_err(Error::Process)
}

#[cfg(feature = "sdk-test")]
pub(super) struct Registration {
    pub(super) parent: u64,
    pub(super) program: u64,
}

#[cfg(feature = "sdk-test")]
pub(super) fn launch_dormant(
    manager: &mut Manager,
    memory: &mut Memory,
    manifest: &[u8],
    name: &str,
    elf: &[u8],
    available: u64,
    registration: Registration,
) -> Result<Pid, Error> {
    let pid = launch(manager, memory, manifest, name, elf, available)?;
    if let Err(error) = manager.table.hold(pid) {
        manager.kill(pid).expect("rollback unheld image");
        manager.wait(memory, pid).expect("reap unheld image");
        return Err(Error::Process(ProcessError::Process(error)));
    }
    let slot = manager.table.slot(pid).expect("held image slot");
    let process = manager.processes[slot].as_mut().expect("held image record");
    process.parent = registration.parent;
    process.program = registration.program;
    Ok(pid)
}
