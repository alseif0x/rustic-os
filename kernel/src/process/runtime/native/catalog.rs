// SPDX-License-Identifier: Apache-2.0
//! Opaque trusted executable catalog. Manifests request features, never mint authority.
use super::super::{application, manager::Manager};
use crate::arch::memory::Memory;
use rustic_abi::runtime::Error;
use rustic_kernel::process::lifecycle::Pid;
macro_rules! image {
    ($name:literal,$suffix:literal) => {
        include_bytes!(concat!(
            env!("RUSTIC_APPLICATION_DIRECTORY"),
            "/",
            $name,
            $suffix
        ))
    };
}
pub(super) fn launch(
    manager: &mut Manager,
    memory: &mut Memory,
    program: u64,
) -> Result<Pid, Error> {
    let (manifest, name, elf): (&[u8], &str, &[u8]) = match program {
        0 => (
            image!("supervisor", ".manifest"),
            "supervisor.elf",
            image!("supervisor", ".elf"),
        ),
        1 => (
            image!("file-server", ".manifest"),
            "file-server.elf",
            image!("file-server", ".elf"),
        ),
        2 => (
            image!("shell", ".manifest"),
            "shell.elf",
            image!("shell", ".elf"),
        ),
        3 => (
            image!("utility", ".manifest"),
            "utility.elf",
            image!("utility", ".elf"),
        ),
        _ => return Err(Error::Invalid),
    };
    application::launch(
        manager,
        memory,
        manifest,
        name,
        elf,
        rustic_abi::application::KNOWN,
    )
    .map_err(|_| Error::Full)
}
