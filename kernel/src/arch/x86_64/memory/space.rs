// SPDX-License-Identifier: Apache-2.0
use super::{
    Error, cpu,
    physical::Physical,
    tables::{self, ADDRESS, HUGE, PRESENT, USER, WRITE},
};
use core::marker::PhantomData;
use rustic_kernel::memory::{PagePermissions, VirtualPage};

/// Owns its root and lower-half tables/data. Higher-half kernel tables are shared
/// and must outlive every child. No borrowed data references escape this interface.
pub(super) struct AddressSpace {
    root: u64,
    kernel: bool,
    _local: PhantomData<*mut ()>,
}

impl AddressSpace {
    pub(super) fn is_active(&self) -> bool {
        self.root == cpu::root()
    }
    pub(super) fn from_kernel(root: u64) -> Self {
        Self {
            root,
            kernel: true,
            _local: PhantomData,
        }
    }

    pub(super) fn child(&self, memory: &mut Physical) -> Result<Self, Error> {
        if !self.kernel {
            return Err(Error::InvalidAddress);
        }
        let root = memory.allocate_zeroed()?;
        for index in 256..512 {
            memory.write(root, index, memory.read(self.root, index));
        }
        Ok(Self {
            root,
            kernel: false,
            _local: PhantomData,
        })
    }

    /// Caller retains only common kernel/stack references across the switch.
    pub(super) unsafe fn activate(&self) -> Result<(), Error> {
        if self.root == 0 {
            return Err(Error::InvalidAddress);
        }
        // SAFETY: Caller guarantees address-space lifetime and compatible stack/code.
        unsafe {
            cpu::activate(self.root);
        }
        Ok(())
    }

    pub(super) fn lookup(&self, memory: &Physical, address: u64) -> Option<tables::Mapping> {
        tables::lookup(memory, self.root, address)
    }

    pub(super) fn map_zeroed(
        &mut self,
        memory: &mut Physical,
        page: VirtualPage,
        permissions: PagePermissions,
    ) -> Result<(), Error> {
        if self.root == 0 {
            return Err(Error::InvalidAddress);
        }
        if !permissions.valid() {
            return Err(Error::WritableExecutable);
        }
        let address = page.address();
        if self.lookup(memory, address).is_some() {
            return Err(Error::AlreadyMapped);
        }
        let frame = memory.allocate_zeroed()?;
        let mut created = [(0, 0, 0); 3];
        let mut count = 0;
        let result = (|| {
            let mut table = self.root;
            for level in (2..=4).rev() {
                let slot = tables::index(address, level);
                let entry = memory.read(table, slot);
                if entry & PRESENT == 0 {
                    let child = memory.allocate_zeroed()?;
                    memory.write(table, slot, child | PRESENT | WRITE | USER);
                    created[count] = (table, slot, child);
                    count += 1;
                    table = child;
                } else {
                    if entry & HUGE != 0 {
                        return Err(Error::CorruptTable);
                    }
                    table = entry & ADDRESS;
                }
            }
            memory.write(
                table,
                tables::index(address, 1),
                frame | tables::flags(permissions),
            );
            Ok(())
        })();
        if result.is_err() {
            for &(parent, slot, child) in created[..count].iter().rev() {
                memory.write(parent, slot, 0);
                memory.release(child)?;
            }
            memory.release(frame)?;
        }
        if cpu::root() == self.root {
            cpu::invalidate(address);
        }
        result
    }

    pub(super) fn unmap(&mut self, memory: &mut Physical, page: VirtualPage) -> Result<(), Error> {
        if self.root == 0 {
            return Err(Error::InvalidAddress);
        }
        let address = page.address();
        let mut path = [(0, 0, 0); 3];
        let mut table = self.root;
        for (depth, level) in (2..=4).rev().enumerate() {
            let slot = tables::index(address, level);
            let entry = memory.read(table, slot);
            if entry & PRESENT == 0 {
                return Err(Error::NotMapped);
            }
            if entry & HUGE != 0 {
                return Err(Error::CorruptTable);
            }
            let child = entry & ADDRESS;
            path[depth] = (table, slot, child);
            table = child;
        }
        let slot = tables::index(address, 1);
        let entry = memory.read(table, slot);
        if entry & PRESENT == 0 {
            return Err(Error::NotMapped);
        }
        memory.write(table, slot, 0);
        if cpu::root() == self.root {
            cpu::invalidate(address);
        }
        memory.release(entry & ADDRESS)?;
        for &(parent, slot, child) in path.iter().rev() {
            if !memory.empty(child) {
                break;
            }
            memory.write(parent, slot, 0);
            memory.release(child)?;
        }
        // Reload also flushes paging-structure caches after freeing empty tables.
        if cpu::root() == self.root {
            // SAFETY: Same live root and upper mappings; only unused lower tables removed.
            unsafe {
                cpu::activate(self.root);
            }
        }
        Ok(())
    }

    pub(super) fn destroy(&mut self, memory: &mut Physical) -> Result<(), Error> {
        if self.kernel || self.root == cpu::root() {
            return Err(Error::ActiveSpace);
        }
        if self.root == 0 {
            return Err(Error::InvalidAddress);
        }
        for slot in 0..256 {
            let entry = memory.read(self.root, slot);
            if entry & PRESENT != 0 {
                release_tree(memory, entry & ADDRESS, 3)?;
            }
        }
        memory.release(self.root)?;
        self.root = 0;
        Ok(())
    }
}

fn release_tree(memory: &mut Physical, table: u64, level: usize) -> Result<(), Error> {
    for slot in 0..512 {
        let entry = memory.read(table, slot);
        if entry & PRESENT == 0 {
            continue;
        }
        if level == 1 {
            memory.release(entry & ADDRESS)?;
        } else {
            if entry & HUGE != 0 {
                return Err(Error::CorruptTable);
            }
            release_tree(memory, entry & ADDRESS, level - 1)?;
        }
    }
    memory.release(table)
}
