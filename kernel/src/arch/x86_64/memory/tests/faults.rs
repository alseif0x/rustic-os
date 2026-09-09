// SPDX-License-Identifier: Apache-2.0
use super::super::bootstrap;
use super::{Memory, page};
use crate::arch::{self, Serial};
use core::fmt::Write;
use rustic_kernel::{boot::BootMode, memory::PagePermissions};

pub(crate) fn fault(memory: &mut Memory, mode: BootMode) -> ! {
    let address = match mode {
        BootMode::MemoryReadOnly => {
            memory
                .kernel
                .map_zeroed(&mut memory.physical, page(), PagePermissions::READ_ONLY)
                .unwrap();
            page().address()
        }
        BootMode::MemoryNx => {
            memory
                .kernel
                .map_zeroed(&mut memory.physical, page(), PagePermissions::READ_WRITE)
                .unwrap();
            let mapping = memory
                .kernel
                .lookup(&memory.physical, page().address())
                .unwrap();
            memory.physical.write(mapping.physical, 0, 0xc3); // RET, if NX were broken.
            page().address()
        }
        BootMode::MemoryUnmapped => {
            memory
                .kernel
                .map_zeroed(&mut memory.physical, page(), PagePermissions::READ_WRITE)
                .unwrap();
            memory.kernel.unmap(&mut memory.physical, page()).unwrap();
            page().address()
        }
        BootMode::MemoryTextAlias => {
            let code = memory
                .kernel
                .lookup(&memory.physical, bootstrap::text_start())
                .unwrap();
            memory.layout.hhdm + code.physical
        }
        BootMode::MemoryGuard => arch::interrupts::emergency_guard(),
        _ => panic!("invalid memory fixture"),
    };
    if let Some(mut serial) = Serial::take() {
        let _ = writeln!(
            serial,
            "RUSTIC MEMORY_FAULT mode={mode:?} address={address:#x}"
        );
        serial.flush();
    }
    // SAFETY: These terminal fixtures intentionally perform forbidden CPU accesses.
    // No invalid Rust references are formed; UD2 detects an unexpectedly allowed access.
    unsafe {
        match mode {
            BootMode::MemoryNx => {
                core::arch::asm!("call {}", "ud2", in(reg) address, options(noreturn))
            }
            BootMode::MemoryReadOnly | BootMode::MemoryTextAlias => {
                core::arch::asm!("mov byte ptr [{}], 0", "ud2", in(reg) address, options(noreturn))
            }
            _ => {
                core::arch::asm!("mov al, byte ptr [{}]", "ud2", in(reg) address, options(noreturn))
            }
        }
    }
}
