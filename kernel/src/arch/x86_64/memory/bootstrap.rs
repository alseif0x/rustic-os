// SPDX-License-Identifier: Apache-2.0
use super::{
    BootMemory, Error, cpu,
    physical::Physical,
    space::AddressSpace,
    tables::{self, ADDRESS, GLOBAL, HUGE, NX, PRESENT, USER},
};
use rustic_kernel::memory::{PAGE_SIZE, PagePermissions};

unsafe extern "C" {
    static __kernel_start: u8;
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_end: u8;
    static __kernel_end: u8;
}

pub(super) fn image_range() -> (u64, u64) {
    (
        core::ptr::addr_of!(__kernel_start) as u64,
        core::ptr::addr_of!(__kernel_end) as u64,
    )
}
pub(super) fn text_start() -> u64 {
    core::ptr::addr_of!(__text_start) as u64
}
pub(super) fn rodata_start() -> u64 {
    core::ptr::addr_of!(__text_end) as u64
}

fn clone_table(memory: &mut Physical, source: u64, level: usize) -> Result<u64, Error> {
    let target = memory.allocate_zeroed()?;
    let first = if level == 4 { 256 } else { 0 };
    for slot in first..512 {
        let entry = memory.read(source, slot);
        if entry & PRESENT == 0 {
            continue;
        }
        let copied = if level == 1 || entry & HUGE != 0 {
            if level == 4 {
                return Err(Error::CorruptTable);
            }
            (entry & !(USER | GLOBAL)) | NX
        } else {
            let child = clone_table(memory, entry & ADDRESS, level - 1)?;
            child | (entry & !ADDRESS & !(USER | NX))
        };
        // Direct-map branches are supervisor and never executable, including aliases.
        memory.write(
            target,
            slot,
            copied | if level == 4 && slot != 511 { NX } else { 0 },
        );
    }
    Ok(target)
}

pub(super) fn build(memory: &mut Physical, layout: BootMemory) -> Result<AddressSpace, Error> {
    cpu::protection()?;
    let root = clone_table(memory, cpu::root(), 4)?;
    let (start, end) = image_range();
    let code = text_start();
    let code_end = rodata_start();
    let rodata_end = core::ptr::addr_of!(__rodata_end) as u64;
    for address in (start..end).step_by(PAGE_SIZE as usize) {
        let permissions = if (code..code_end).contains(&address) {
            PagePermissions::CODE
        } else if address < rodata_end {
            PagePermissions::READ_ONLY
        } else {
            PagePermissions::READ_WRITE
        };
        tables::protect(memory, root, address, Some(permissions))?;
        let alias = layout.hhdm + layout.physical_base + address - start;
        tables::protect(
            memory,
            root,
            alias,
            Some(PagePermissions {
                executable: false,
                ..permissions
            }),
        )?;
    }
    for guard in [
        crate::arch::interrupts::emergency_guard(),
        crate::arch::interrupts::user_guard(),
    ] {
        tables::protect(memory, root, guard, None)?;
        tables::protect(
            memory,
            root,
            layout.hhdm + layout.physical_base + guard - start,
            None,
        )?;
    }
    Ok(AddressSpace::from_kernel(root))
}
