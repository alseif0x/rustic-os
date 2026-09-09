// SPDX-License-Identifier: Apache-2.0
//! Process-owned mappings. Loading writes only to an inactive, newly owned page.
use super::{AddressSpace, Error, Memory};
use crate::arch::interrupts::{self, Frame, Mask, UserEvent};
use rustic_kernel::memory::{PAGE_SIZE, PagePermissions, VirtualPage};

pub(crate) struct UserSpace {
    inner: AddressSpace,
}

impl Memory {
    pub(crate) fn run_user(&self, space: &UserSpace, frame: &mut Frame) -> UserEvent {
        let _mask = Mask::acquire();
        // SAFETY: Memory owns the common upper mappings and this live private
        // root; the borrowed frame lives in kernel memory. IF=0 excludes another
        // switch. The bridge returns synchronously without lower-half references.
        unsafe {
            space.activate();
            let event = interrupts::run_user(frame);
            self.activate_kernel();
            event
        }
    }
    pub(crate) fn free_frames(&self) -> usize {
        self.physical.frames.free_count()
    }

    pub(crate) fn create_user(&mut self) -> Result<UserSpace, Error> {
        Ok(UserSpace {
            inner: self.kernel.child(&mut self.physical)?,
        })
    }

    pub(crate) fn load_page(
        &mut self,
        space: &mut UserSpace,
        address: u64,
        permissions: PagePermissions,
        offset: usize,
        data: &[u8],
    ) -> Result<(), Error> {
        if !permissions.user
            || offset
                .checked_add(data.len())
                .is_none_or(|end| end > PAGE_SIZE as usize)
            || space.inner.is_active()
        {
            return Err(Error::InvalidAddress);
        }
        let page = VirtualPage::new(address).ok_or(Error::InvalidAddress)?;
        space
            .inner
            .map_zeroed(&mut self.physical, page, permissions)?;
        let mapping = space
            .inner
            .lookup(&self.physical, address)
            .ok_or(Error::NotMapped)?;
        self.physical
            .initialize_bytes(mapping.physical, offset, data);
        Ok(())
    }

    pub(crate) fn destroy_user(&mut self, space: &mut UserSpace) -> Result<(), Error> {
        space.inner.destroy(&mut self.physical)
    }

    /// Both roots retain the same upper kernel mappings; caller owns the CPU and
    /// retains no lower-half references across execution. User cannot access them.
    unsafe fn activate_kernel(&self) {
        // SAFETY: Forward the caller's CPU/reference invariants to the root switch.
        unsafe { self.kernel.activate().expect("live kernel root") }
    }
}

impl UserSpace {
    /// Caller keeps this space alive until returning to the common kernel root,
    /// has IF=0, and holds only higher-half references across the switch.
    unsafe fn activate(&self) {
        // SAFETY: The process owner guarantees the documented switch invariants.
        unsafe { self.inner.activate().expect("live user root") }
    }
}
