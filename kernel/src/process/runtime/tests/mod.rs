// SPDX-License-Identifier: Apache-2.0
mod execution;
mod faults;
mod loading;
use super::{Error, manager::Manager};
use crate::arch::{Serial, memory::Memory};
use core::fmt::Write;
use rustic_kernel::process::lifecycle::{Exit, Pid, State};

core::arch::global_asm!(include_str!("../fixture.S"));
unsafe extern "C" {
    static rustic_user_elf: u8;
    static rustic_user_elf_end: u8;
    static rustic_victim_elf: u8;
    static rustic_victim_elf_end: u8;
}

fn image(victim: bool) -> &'static [u8] {
    let (start, end) = if victim {
        (
            &raw const rustic_victim_elf,
            &raw const rustic_victim_elf_end,
        )
    } else {
        (&raw const rustic_user_elf, &raw const rustic_user_elf_end)
    };
    // SAFETY: Assembly emits contiguous immutable ELF bytes in resident rodata;
    // end belongs to that same object, both symbols survive every CR3 switch.
    unsafe { core::slice::from_raw_parts(start, end as usize - start as usize) }
}

fn drive(manager: &mut Manager, memory: &Memory, pid: Pid) {
    for _ in 0..128 {
        if matches!(manager.state(pid).unwrap(), State::Exited(_)) {
            return;
        }
        assert!(manager.step(memory).unwrap().is_some());
    }
    panic!("process failed to terminate within event budget");
}

pub(crate) fn verify(memory: &mut Memory) {
    let before = memory.free_frames();
    let mut manager = Manager::new();
    let peak_frames = loading::verify(&mut manager, memory);
    let preemptions = execution::verify(&mut manager, memory);
    let faults = faults::verify(&mut manager, memory);
    assert_eq!(memory.free_frames(), before);
    let mut serial = Serial::take().expect("process diagnostic owner");
    writeln!(serial, "RUSTIC PROCESS_MEMORY slots=4 peak_frames={peak_frames} metadata_bytes={} entry_stack_bytes=20480 oom_cases=3", core::mem::size_of::<Manager>()).unwrap();
    writeln!(serial, "RUSTIC PROCESS verified=1 ring=3 elf=1 preemptions={preemptions} isolated_faults={faults} repeats=16 reclaimed=1 abi=65536 free_before={before} free_after={}", memory.free_frames()).unwrap();
    serial.flush();
}
