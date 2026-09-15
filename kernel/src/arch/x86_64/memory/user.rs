// SPDX-License-Identifier: Apache-2.0
//! Process-owned mappings. Loading writes only to an inactive, newly owned page.
use super::{AddressSpace, Error, Memory};
use crate::arch::interrupts::{self, Frame, Mask, UserEvent};
use rustic_kernel::memory::{PAGE_SIZE, PagePermissions, VirtualPage};

pub(crate) struct UserSpace {
    pub(super) inner: AddressSpace,
}

/// Page `index` of a run starting at `base`, rejecting overflow and policy.
fn page_of(base: u64, index: u64) -> Result<VirtualPage, Error> {
    let address = index
        .checked_mul(PAGE_SIZE)
        .and_then(|offset| base.checked_add(offset))
        .ok_or(Error::InvalidAddress)?;
    VirtualPage::new(address).ok_or(Error::InvalidAddress)
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

    /// Map a run of zeroed, never executable user pages into an inactive space.
    /// A failure leaves the space exactly as it was: every page mapped by this
    /// call is unmapped again and its frame returned before reporting.
    pub(crate) fn map_user_pages(
        &mut self,
        space: &mut UserSpace,
        base: u64,
        pages: u64,
        writable: bool,
    ) -> Result<(), Error> {
        if space.inner.is_active() {
            return Err(Error::InvalidAddress);
        }
        let permissions = PagePermissions {
            writable,
            executable: false,
            user: true,
        };
        let mut mapped = 0;
        let result = (|| {
            while mapped < pages {
                let page = page_of(base, mapped)?;
                space
                    .inner
                    .map_zeroed(&mut self.physical, page, permissions)?;
                mapped += 1;
            }
            Ok(())
        })();
        if result.is_err() && mapped > 0 {
            self.unmap_user_pages(space, base, mapped)
                .expect("rollback pages mapped by this call");
        }
        result
    }

    /// Unmap a run this kernel previously mapped for the process, returning its
    /// frames. The caller owns the range; an unmapped page is a kernel error.
    pub(crate) fn unmap_user_pages(
        &mut self,
        space: &mut UserSpace,
        base: u64,
        pages: u64,
    ) -> Result<(), Error> {
        if space.inner.is_active() {
            return Err(Error::InvalidAddress);
        }
        for index in 0..pages {
            let page = page_of(base, index)?;
            space.inner.unmap(&mut self.physical, page)?;
        }
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
