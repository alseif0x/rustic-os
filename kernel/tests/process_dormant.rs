// SPDX-License-Identifier: Apache-2.0
use rustic_kernel::process::lifecycle::{CAPACITY, Error, Exit, State, Table};
#[test]
fn dormant_children_are_not_scheduled_and_capacity_is_recovered() {
    let mut table = Table::new();
    let mut pids = Vec::new();
    for _ in 0..CAPACITY {
        let (_, pid) = table.create().unwrap();
        table.hold(pid).unwrap();
        pids.push(pid);
    }
    assert_eq!(table.schedule(), Ok(None));
    assert_eq!(table.create(), Err(Error::Full));
    table.start(pids[3]).unwrap();
    assert_eq!(table.schedule().unwrap().unwrap().1, pids[3]);
    assert_eq!(table.start(pids[3]), Err(Error::NotRunning));
    for pid in &pids {
        table.finish(*pid, Exit::Killed).unwrap();
        assert_eq!(table.reap(*pid), Ok(Exit::Killed));
    }
    let (_, fresh) = table.create().unwrap();
    assert!(fresh.0 > pids.last().unwrap().0);
    assert_eq!(table.state(pids[3]), Err(Error::Unknown));
    assert_eq!(table.state(fresh), Ok(State::Ready));
}
