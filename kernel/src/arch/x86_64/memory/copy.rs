// SPDX-License-Identifier: Apache-2.0
//! Validate the whole range before touching bytes; no user virtual references.
use super::{Error, Memory, UserSpace};
use rustic_kernel::memory::PAGE_SIZE;

impl Memory {
    pub(crate) fn validate_buffer(
        &self,
        space: &UserSpace,
        address: u64,
        length: usize,
        write: bool,
    ) -> Result<(), Error> {
        let end = address
            .checked_add(length as u64)
            .ok_or(Error::InvalidAddress)?;
        if length == 0
            || length > 4096
            || address < PAGE_SIZE
            || end > 1 << 47
            || space.inner.is_active()
        {
            return Err(Error::InvalidAddress);
        }
        for page in (address & !(PAGE_SIZE - 1)..end).step_by(PAGE_SIZE as usize) {
            let mapping = space
                .inner
                .lookup(&self.physical, page)
                .ok_or(Error::InvalidAddress)?;
            if !mapping.user
                || (write && !mapping.writable)
                || !self
                    .physical
                    .frames
                    .is_allocated(mapping.physical & !(PAGE_SIZE - 1))
            {
                return Err(Error::InvalidAddress);
            }
        }
        Ok(())
    }
    pub(crate) fn copy_from_user(
        &self,
        space: &UserSpace,
        address: u64,
        bytes: &mut [u8],
    ) -> Result<(), Error> {
        self.validate_buffer(space, address, bytes.len(), false)?;
        let mut done = 0;
        while done < bytes.len() {
            let mapping = space
                .inner
                .lookup(&self.physical, address + done as u64)
                .unwrap();
            let offset = (mapping.physical % PAGE_SIZE) as usize;
            let count = (PAGE_SIZE as usize - offset).min(bytes.len() - done);
            self.physical.read_bytes(
                mapping.physical - offset as u64,
                offset,
                &mut bytes[done..done + count],
            );
            done += count;
        }
        Ok(())
    }
    pub(crate) fn copy_to_user(
        &mut self,
        space: &UserSpace,
        address: u64,
        bytes: &[u8],
    ) -> Result<(), Error> {
        self.validate_buffer(space, address, bytes.len(), true)?;
        let mut done = 0;
        while done < bytes.len() {
            let mapping = space
                .inner
                .lookup(&self.physical, address + done as u64)
                .unwrap();
            let offset = (mapping.physical % PAGE_SIZE) as usize;
            let count = (PAGE_SIZE as usize - offset).min(bytes.len() - done);
            self.physical.initialize_bytes(
                mapping.physical - offset as u64,
                offset,
                &bytes[done..done + count],
            );
            done += count;
        }
        Ok(())
    }
}
