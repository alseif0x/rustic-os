// SPDX-License-Identifier: Apache-2.0
mod buffers;
mod exchange;
mod waits;
use super::{Exit, Manager, Memory, Pid, State, drive};
use rustic_abi::ipc::{self as abi, Error};

core::arch::global_asm!(include_str!("fixture.S"));
unsafe extern "C" {
    static rustic_ipc_elf: u8;
    static rustic_ipc_elf_end: u8;
}
fn image() -> &'static [u8] {
    let start = &raw const rustic_ipc_elf;
    let end = &raw const rustic_ipc_elf_end;
    // SAFETY: Contiguous immutable ELF in resident kernel rodata, kept alive forever.
    unsafe { core::slice::from_raw_parts(start, end as usize - start as usize) }
}
fn create(manager: &mut Manager, memory: &mut Memory) -> Pid {
    manager.create(memory, image(), [0; 3]).unwrap()
}
fn packet() -> [u8; 32] {
    let mut bytes = [0; 32];
    bytes[..2].copy_from_slice(&1u16.to_le_bytes());
    bytes[2..4].copy_from_slice(&1u16.to_le_bytes());
    bytes[4..8].copy_from_slice(&8u32.to_le_bytes());
    bytes[8..16].copy_from_slice(&123u64.to_le_bytes());
    bytes[24..].copy_from_slice(&0x42u64.to_le_bytes());
    bytes
}
fn cleanup(manager: &mut Manager, memory: &mut Memory, pid: Pid) {
    if !matches!(manager.state(pid).unwrap(), State::Exited(_)) {
        manager.kill(pid).unwrap();
    }
    assert!(manager.wait(memory, pid).unwrap().is_some());
}
pub(super) fn verify(manager: &mut Manager, memory: &mut Memory) {
    let before = memory.free_frames();
    exchange::verify(manager, memory);
    let rejected = buffers::verify(manager, memory);
    waits::verify(manager, memory);
    assert_eq!(manager.broker.counts(), (0, 0));
    assert_eq!(memory.free_frames(), before);
    let mut serial = crate::arch::Serial::take().unwrap();
    use core::fmt::Write;
    writeln!(serial, "RUSTIC IPC verified=1 ring=3 exchanges=16 rejected={rejected} wait=1 cancel=1 close=1 death=1 transfer=1 attenuation=1 stale=1 cross_page=1 atomic_copy=1 version=1 channels=0 handles=0 free_before={before} free_after={}", memory.free_frames()).unwrap();
    serial.flush();
}
