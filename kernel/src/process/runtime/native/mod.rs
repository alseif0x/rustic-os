// SPDX-License-Identifier: Apache-2.0
//! Boot capability root and bounded runtime mechanisms. No filesystem/session policy.
mod catalog;
pub(super) mod console;
mod control;
mod session;
pub(super) mod wait;
use super::{manager::Manager, record::Process, syscall::Action};
use crate::arch::{interrupts, memory::Memory};
pub(super) use session::Session;
pub(super) fn dispatch(
    number: u64,
    process: &mut Process,
    broker: &rustic_kernel::ipc::Broker,
    owner: u64,
    console: u64,
    memory: &mut Memory,
) -> Action {
    use rustic_abi::runtime::*;
    match number {
        CLOCK if process.frame.arguments() == [0; 3] => Action::Return(interrupts::ticks()),
        WAIT_SET => wait::dispatch(process, broker, owner, memory),
        CONSOLE_WRITE | CONSOLE_READ | CONSOLE_WAIT => {
            console::dispatch(number, process, owner, console, memory)
        }
        _ => Action::Return(Error::Invalid.code()),
    }
}
pub(crate) fn run(
    memory: &mut Memory,
    interrupts: &mut interrupts::Controller,
    initialize: bool,
) -> ! {
    let before = memory.free_frames();
    let mut manager = Manager::new();
    manager
        .block
        .open(memory)
        .expect("native managed block device");
    let pid = catalog::launch(&mut manager, memory, 0).expect("supervisor executable");
    manager.session.supervisor = pid.0;
    manager.bootstrap(pid, [initialize as u64, 0, 0]);
    loop {
        if manager.session.shutdown {
            break;
        }
        if matches!(
            manager.state(pid),
            Ok(rustic_kernel::process::lifecycle::State::Exited(_))
        ) {
            panic!("native supervisor exited: {:?}", manager.state(pid));
        }
        if manager.step(memory).expect("native scheduling").is_none() {
            interrupts.idle();
        }
    }
    for slot in 0..rustic_kernel::process::lifecycle::CAPACITY {
        if let Some(pid) = manager.table.pid_at(slot) {
            if !matches!(
                manager.state(pid),
                Ok(rustic_kernel::process::lifecycle::State::Exited(_))
            ) {
                manager.kill(pid).unwrap();
            }
            manager.wait(memory, pid).unwrap();
        }
    }
    for _ in 0..1000 {
        manager.block.tick(memory);
        if manager.block.broker.counts() == (0, 0) {
            break;
        }
        interrupts.idle();
    }
    manager
        .block
        .shutdown(memory)
        .expect("native shutdown releases device");
    assert_eq!(manager.broker.counts(), (0, 0));
    assert!(manager.processes.iter().all(Option::is_none));
    assert_eq!(memory.free_frames(), before);
    use core::fmt::Write;
    if let Some(mut serial) = crate::arch::Serial::take() {
        writeln!(
            serial,
            "RUSTIC TERMINAL stopped=1 reclaimed=1 free_before={before} free_after={}",
            memory.free_frames()
        )
        .unwrap();
        serial.flush();
    }
    crate::arch::test_exit(0x10)
}
