// SPDX-License-Identifier: Apache-2.0
//! R0 memory owner. Physical bookkeeping, table mechanics and fixtures stay separate.
mod bootstrap;
#[cfg(feature = "sdk-test")]
mod buffer;
mod copy;
mod dma;
#[cfg(feature = "sdk-test")]
pub(crate) use buffer::KernelBuffer;
pub(crate) use dma::DmaRegion;
mod cpu;
mod physical;
mod space;
mod tables;
mod tests;
mod user;

pub(crate) use user::UserSpace;

use core::marker::PhantomData;
use physical::Physical;
use rustic_kernel::boot::validate_map;
use rustic_kernel::memory::FrameError;
use space::AddressSpace;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Frames(FrameError),
    InvalidAddress,
    AlreadyMapped,
    NotMapped,
    WritableExecutable,
    CorruptTable,
    ActiveSpace,
    UnsupportedCpu,
    AlreadyInitialized,
}

impl From<FrameError> for Error {
    fn from(value: FrameError) -> Self {
        Self::Frames(value)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct BootMemory {
    pub(crate) hhdm: u64,
    pub(crate) physical_base: u64,
    pub(crate) virtual_base: u64,
}

pub(crate) struct Memory {
    physical: Physical,
    kernel: AddressSpace,
    layout: BootMemory,
    /// Firmware-reported usable bytes, kept for the boot report so the managed
    /// count and the amount left reserved can be compared truthfully.
    usable: u64,
    _local: PhantomData<*mut ()>,
}

impl Memory {
    pub(crate) fn initialize(
        layout: BootMemory,
        entries: impl Iterator<Item = (u64, u64, bool)> + Clone,
    ) -> Result<Self, Error> {
        // The validated summary supplies usable bytes with checked arithmetic, so
        // the report never sums raw firmware lengths itself.
        let usable = validate_map(entries.clone())
            .map_err(|_| Error::Frames(FrameError::InvalidMap))?
            .usable_bytes;
        let mut physical = Physical::initialize(layout, entries)?;
        let kernel = bootstrap::build(&mut physical, layout)?;
        // SAFETY: Bootstrap copied all needed higher-half mappings, made executable
        // text accessible, and kept the current stack/IDT/TSS mapped. One CPU only.
        unsafe {
            kernel.activate()?;
        }
        Ok(Self {
            physical,
            kernel,
            layout,
            usable,
            _local: PhantomData,
        })
    }

    pub(crate) fn verify(&mut self) {
        tests::verify(self);
    }
    pub(crate) fn verify_frame_budget(&mut self, remaining: usize, test: impl FnOnce(&mut Self)) {
        tests::with_free_frames(self, remaining, test);
    }
    pub(crate) fn fault(&mut self, mode: rustic_kernel::boot::BootMode) -> ! {
        tests::fault(self, mode)
    }
}
