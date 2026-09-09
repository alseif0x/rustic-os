// SPDX-License-Identifier: Apache-2.0
//! Explicitly disable privilege-entry paths outside the R0 INT ABI.

pub(super) fn initialize() {
    let low: u32;
    let high: u32;
    let cr4: u64;
    // SAFETY: Unique bootstrap CPU at ring 0, IF=0. EFER exists on x86_64;
    // preserve long-mode/NX bits while clearing SCE. Do not inherit a firmware
    // syscall target. Clear optional user FS/GS-base writes/performance access.
    unsafe {
        core::arch::asm!("rdmsr", in("ecx") 0xc000_0080u32, out("eax") low, out("edx") high, options(nostack));
        core::arch::asm!("wrmsr", in("ecx") 0xc000_0080u32, in("eax") (low & !1), in("edx") high, options(nostack));
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
        core::arch::asm!("mov cr4, {}", in(reg) (cr4 & !((1 << 16) | (1 << 8))), options(nostack));
    }
    if core::arch::x86_64::__cpuid(1).edx & (1 << 11) != 0 {
        // SAFETY: SEP advertises these MSRs. A zero SYSENTER_CS rejects entry;
        // stack/target are cleared too, before any user instruction can execute.
        for msr in [0x174u32, 0x175, 0x176] {
            // SAFETY: SEP was checked; each listed MSR accepts a zero value.
            unsafe {
                core::arch::asm!("wrmsr", in("ecx") msr, in("eax") 0u32, in("edx") 0u32, options(nostack))
            }
        }
    }
}
