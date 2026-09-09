// SPDX-License-Identifier: Apache-2.0
use super::Error;

pub(super) fn root() -> u64 {
    let value: u64;
    // SAFETY: R0 kernel runs at ring 0. No PCID is used.
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value & 0x000f_ffff_ffff_f000
}

pub(super) fn protection() -> Result<(), Error> {
    let cr4: u64;
    let mut cr0: u64;
    let low: u32;
    let high: u32;
    // CPUID is available on x86_64; query only supported leaves before enabling NX.
    let extended = core::arch::x86_64::__cpuid(0x8000_0000);
    if extended.eax < 0x8000_0001 || core::arch::x86_64::__cpuid(0x8000_0001).edx & (1 << 20) == 0 {
        return Err(Error::UnsupportedCpu);
    }
    // SAFETY: Ring 0 bootstrap; refuse five-level paging/PCID. PGE is disabled to
    // flush old global translations; our new tables do not install global entries.
    unsafe {
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags));
        if cr4 & ((1 << 12) | (1 << 17)) != 0 {
            return Err(Error::UnsupportedCpu);
        }
        core::arch::asm!("mov cr4, {}", in(reg) (cr4 & !(1 << 7)), options(nostack));
        core::arch::asm!("rdmsr", in("ecx") 0xc000_0080u32, out("eax") low, out("edx") high, options(nostack));
        core::arch::asm!("wrmsr", in("ecx") 0xc000_0080u32, in("eax") (low | (1 << 11)), in("edx") high, options(nostack));
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
        cr0 |= 1 << 16;
        core::arch::asm!("mov cr0, {}", in(reg) cr0, options(nostack));
    }
    Ok(())
}

/// Caller must preserve executable code, stack and interrupt mappings across switch.
pub(super) unsafe fn activate(root: u64) {
    // SAFETY: Caller owns the aligned root and all required mappings. No PCID/PGE.
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) root, options(nostack));
    }
}

pub(super) fn invalidate(address: u64) {
    // SAFETY: Ring 0; single CPU, caller cleared/changed this entry before flushing.
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) address, options(nostack, preserves_flags));
    }
}
