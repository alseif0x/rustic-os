// SPDX-License-Identifier: Apache-2.0
use super::{Error, physical::Physical};
use rustic_kernel::memory::{PagePermissions, canonical};

pub(super) const ADDRESS: u64 = 0x000f_ffff_ffff_f000;
pub(super) const PRESENT: u64 = 1;
pub(super) const WRITE: u64 = 2;
pub(super) const USER: u64 = 4;
pub(super) const HUGE: u64 = 1 << 7;
pub(super) const GLOBAL: u64 = 1 << 8;
pub(super) const NX: u64 = 1 << 63;

pub(super) fn index(address: u64, level: usize) -> usize {
    ((address >> (12 + (level - 1) * 9)) & 511) as usize
}
pub(super) fn flags(permissions: PagePermissions) -> u64 {
    PRESENT
        | if permissions.writable { WRITE } else { 0 }
        | if permissions.user { USER } else { 0 }
        | if permissions.executable { 0 } else { NX }
}

#[derive(Debug)]
pub(super) struct Mapping {
    pub(super) physical: u64,
    pub(super) writable: bool,
    pub(super) executable: bool,
    pub(super) user: bool,
}

pub(super) fn lookup(memory: &Physical, root: u64, address: u64) -> Option<Mapping> {
    if root == 0 || !canonical(address) {
        return None;
    }
    let mut table = root;
    let (mut writable, mut user, mut executable) = (true, true, true);
    for level in (1..=4).rev() {
        let entry = memory.read(table, index(address, level));
        if entry & PRESENT == 0 {
            return None;
        }
        writable &= entry & WRITE != 0;
        user &= entry & USER != 0;
        executable &= entry & NX == 0;
        if level == 1 || entry & HUGE != 0 {
            if level == 4 {
                return None;
            }
            let mask = (1u64 << (12 + (level - 1) * 9)) - 1;
            return Some(Mapping {
                physical: (entry & ADDRESS & !mask) | (address & mask),
                writable,
                executable,
                user,
            });
        }
        table = entry & ADDRESS;
    }
    None
}

/// Split a cloned 1-GiB/2-MiB leaf, preserving cache attributes and physical pages.
pub(super) fn split(
    memory: &mut Physical,
    table: u64,
    slot: usize,
    level: usize,
) -> Result<u64, Error> {
    if !(2..=3).contains(&level) {
        return Err(Error::CorruptTable);
    }
    let entry = memory.read(table, slot);
    let child = memory.allocate_zeroed()?;
    let size = 1u64 << (12 + (level - 1) * 9);
    let base = entry & ADDRESS & !(size - 1);
    let mut attrs = entry & !ADDRESS;
    if level == 2 {
        attrs &= !HUGE;
    }
    if entry & (1 << 12) != 0 {
        attrs |= if level == 2 { 1 << 7 } else { 1 << 12 };
    }
    for i in 0..512 {
        memory.write(child, i, (base + i as u64 * (size / 512)) | attrs);
    }
    memory.write(table, slot, child | PRESENT | WRITE);
    Ok(child)
}

/// Bootstrap only: all tables are private and inactive; no TLB flush is required.
pub(super) fn protect(
    memory: &mut Physical,
    root: u64,
    address: u64,
    permissions: Option<PagePermissions>,
) -> Result<(), Error> {
    let mut table = root;
    for level in (2..=4).rev() {
        let slot = index(address, level);
        let entry = memory.read(table, slot);
        if entry & PRESENT == 0 {
            return Err(Error::NotMapped);
        }
        table = if entry & HUGE != 0 {
            split(memory, table, slot, level)?
        } else {
            entry & ADDRESS
        };
    }
    let slot = index(address, 1);
    let entry = memory.read(table, slot);
    if entry & PRESENT == 0 {
        return Err(Error::NotMapped);
    }
    memory.write(
        table,
        slot,
        permissions.map_or(0, |p| (entry & ADDRESS) | flags(p) | (entry & 0x98)),
    );
    Ok(())
}
