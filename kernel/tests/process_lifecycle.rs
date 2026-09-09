// SPDX-License-Identifier: Apache-2.0
use rustic_kernel::process::lifecycle::{CAPACITY, Error, Exit, State, Table};

#[test]
fn blocked_tasks_are_skipped_and_woken_exactly_once() {
    let mut table = Table::new();
    let (_, pid) = table.create().unwrap();
    assert_eq!(table.block(pid), Err(Error::NotRunning));
    table.schedule().unwrap();
    table.block(pid).unwrap();
    assert_eq!(table.schedule(), Ok(None));
    table.wake(pid).unwrap();
    assert_eq!(table.wake(pid), Err(Error::NotBlocked));
    table.schedule().unwrap();
    table.block(pid).unwrap();
    table.finish(pid, Exit::Killed).unwrap();
    assert_eq!(table.reap(pid), Ok(Exit::Killed));
}

#[test]
fn round_robin_requires_suspend_and_skips_exited_slots() {
    let mut table = Table::new();
    let (_, a) = table.create().unwrap();
    let (_, b) = table.create().unwrap();
    assert_eq!(table.schedule().unwrap().unwrap().1, a);
    assert_eq!(table.schedule(), Err(Error::Running));
    table.suspend(a).unwrap();
    assert_eq!(table.schedule().unwrap().unwrap().1, b);
    table.finish(b, Exit::Code(7)).unwrap();
    assert_eq!(table.schedule().unwrap().unwrap().1, a);
    table.finish(a, Exit::Killed).unwrap();
    assert_eq!(table.schedule(), Ok(None));
    assert_eq!(table.reap(b), Ok(Exit::Code(7)));
    assert_eq!(table.reap(b), Err(Error::Unknown));
}

#[test]
fn capacity_and_repeated_reaping_do_not_reuse_identity() {
    let mut table = Table::new();
    let mut ids = Vec::new();
    for _ in 0..CAPACITY {
        ids.push(table.create().unwrap().1);
    }
    assert_eq!(table.create(), Err(Error::Full));
    let old = ids[0];
    assert_eq!(table.reap(old), Err(Error::NotExited));
    for _ in 0..1000 {
        let pid = ids[0];
        table.finish(pid, Exit::Killed).unwrap();
        assert_eq!(table.state(pid), Ok(State::Exited(Exit::Killed)));
        assert!(table.finish(pid, Exit::Killed).is_err());
        table.reap(pid).unwrap();
        let next = table.create().unwrap().1;
        assert!(next.0 > pid.0);
        assert_eq!(table.state(pid), Err(Error::Unknown));
        ids[0] = next;
    }
}
