// SPDX-License-Identifier: Apache-2.0
use super::{Exit, Manager, Memory, State, support::*};
use rustic_abi::block::{Error, READ, Status};
use rustic_kernel::{block::access::Grant, process::lifecycle::Pid};

pub(super) fn verify(manager: &mut Manager, memory: &mut Memory, stats: &mut Stats, control: Pid) {
    let (pid, _) = launch(manager, memory, stats, 1, 0);
    drive(manager, memory, &[pid]);
    let rejected = reap(manager, memory, pid, 1);
    assert_eq!(rejected, 32);
    stats.rejected += rejected;
    // Feature metadata alone grants nothing; a live foreign token also grants nothing.
    let foreign = manager
        .grant_block(
            control,
            Grant {
                first: 0,
                sectors: CAPACITY,
                rights: READ,
            },
        )
        .unwrap();
    for handle in [0, foreign] {
        let pid = create(manager, memory, stats);
        manager.bootstrap(pid, [handle, 2, 0]);
        drive(manager, memory, &[pid]);
        assert_eq!(reap(manager, memory, pid, 2), 5);
        stats.rejected += 5;
    }
    manager.block.broker.close(control.0, foreign).unwrap();
    let pid = create(manager, memory, stats);
    let handle = manager
        .grant_block(
            pid,
            Grant {
                first: 8,
                sectors: 2,
                rights: READ,
            },
        )
        .unwrap();
    manager.bootstrap(pid, [handle, 3, 0]);
    drive(manager, memory, &[pid]);
    assert_eq!(reap(manager, memory, pid, 3), 4);
    stats.rejected += 4;
    for status in [Status::Io, Status::Timeout] {
        manager.block.reject_next = status == Status::Io;
        manager.block.hold_next = status == Status::Timeout;
        let (pid, _) = launch(manager, memory, stats, 5, status as u64);
        drive(manager, memory, &[pid]);
        assert_eq!(reap(manager, memory, pid, 5), 1);
        stats.lifecycle += 1;
    }
    for role in [9, 10] {
        manager.block.hold_next = true;
        let (pid, _) = launch(manager, memory, stats, role, 0);
        drive(manager, memory, &[pid]);
        assert_eq!(reap(manager, memory, pid, role), 1);
        drain(manager, memory);
        stats.lifecycle += 1;
    }
    queued_cancel(manager, memory, stats);
    pressure_and_death(manager, memory, stats, control);
    assert_eq!(manager.block.broker.counts(), (0, 0));
}

fn queued_cancel(manager: &mut Manager, memory: &mut Memory, stats: &mut Stats) {
    manager.block.hold_next = true;
    let (owner, _) = launch(manager, memory, stats, 4, 0);
    // Let the holder reach its report before introducing another caller.
    for _ in 0..64 {
        manager.step(memory).unwrap();
        if manager.process(owner).unwrap().reports != 0 {
            break;
        }
    }
    assert!(manager.block.broker.active().is_some());
    let (pid, _) = launch(manager, memory, stats, 8, 0);
    drive(manager, memory, &[pid]);
    assert_eq!(reap(manager, memory, pid, 8), 1);
    manager.kill(owner).unwrap();
    assert_eq!(manager.wait(memory, owner).unwrap(), Some(Exit::Killed));
    drain(manager, memory);
    stats.lifecycle += 1;
}

fn pressure_and_death(manager: &mut Manager, memory: &mut Memory, stats: &mut Stats, control: Pid) {
    let before_progress = manager.process(control).unwrap().preemptions;
    manager.block.hold_next = true;
    let (owner, stale) = launch(manager, memory, stats, 4, 0);
    let (writer, _) = launch(manager, memory, stats, 6, 0);
    let (full, _) = launch(manager, memory, stats, 7, 0);
    for _ in 0..128 {
        manager.step(memory).unwrap();
        if manager.process(writer).unwrap().reports != 0 {
            break;
        }
    }
    assert_eq!(manager.process(writer).unwrap().last_report, 0xee);
    assert!(manager.block.broker.active().is_some());
    assert_eq!(manager.block.broker.counts().1, 2);
    let before_reap = memory.free_frames();
    manager.kill(owner).unwrap();
    assert_eq!(manager.wait(memory, owner).unwrap(), Some(Exit::Killed));
    assert!(memory.free_frames() > before_reap);
    // DMA still exists while user frames can be reused by a new process incarnation.
    assert!(manager.block.broker.active().is_some());
    assert_eq!(
        manager.grant_block(
            owner,
            Grant {
                first: 0,
                sectors: 1,
                rights: READ
            }
        ),
        Err(Error::Handle)
    );
    let replacement = create(manager, memory, stats);
    manager.bootstrap(replacement, [stale, 2, 0]);
    drive(manager, memory, &[writer, full, replacement]);
    assert_eq!(reap(manager, memory, writer, 6), 1);
    assert_eq!(reap(manager, memory, full, 7), 1);
    assert_eq!(reap(manager, memory, replacement, 2), 5);
    assert!(manager.process(control).unwrap().preemptions > before_progress);
    assert!(matches!(manager.state(control).unwrap(), State::Ready));
    stats.rejected += 7; // Full queue, stale owner grant, five stale-handle operations.
    stats.lifecycle += 1;
}
