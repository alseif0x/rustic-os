// SPDX-License-Identifier: Apache-2.0
use crate::arch::memory;
use rustic_kernel::process::{elf, lifecycle};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Error {
    Elf(elf::Error),
    Memory(memory::Error),
    Process(lifecycle::Error),
}

impl From<lifecycle::Error> for Error {
    fn from(error: lifecycle::Error) -> Self {
        Self::Process(error)
    }
}
