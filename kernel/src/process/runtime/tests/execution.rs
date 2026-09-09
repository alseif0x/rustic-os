// SPDX-License-Identifier: Apache-2.0
use super::*;

pub(super) fn verify(manager: &mut Manager, memory: &mut Memory) -> u64 {
    let before = memory.free_frames();
    let spinner = manager.create(memory, image(false), [1, 0, 111]).unwrap();
    let worker = manager.create(memory, image(false), [0, 0, 222]).unwrap();
    assert_eq!(manager.wait(memory, worker).unwrap(), None);
    drive(manager, memory, worker);
    assert_eq!(manager.state(spinner).unwrap(), State::Ready);
    let preemptions = manager.process(spinner).unwrap().preemptions;
    assert!(manager.process(spinner).unwrap().fixture_progress() > 0);
    assert!(
        preemptions >= 2,
        "noncooperating task must be timer-preempted"
    );
    assert_eq!(manager.process(worker).unwrap().last_report, 222);
    assert_eq!(manager.wait(memory, worker).unwrap(), Some(Exit::Code(222)));
    manager.kill(spinner).unwrap();
    assert_eq!(manager.wait(memory, spinner).unwrap(), Some(Exit::Killed));
    assert!(manager.wait(memory, spinner).is_err());
    assert_eq!(memory.free_frames(), before);
    for value in 1..=16 {
        let pid = manager.create(memory, image(false), [0, 0, value]).unwrap();
        assert!(pid.0 > worker.0);
        drive(manager, memory, pid);
        assert_eq!(manager.wait(memory, pid).unwrap(), Some(Exit::Code(value)));
        assert_eq!(memory.free_frames(), before);
    }
    let quota = manager.create(memory, image(false), [10, 0, 42]).unwrap();
    drive(manager, memory, quota);
    assert_eq!(manager.process(quota).unwrap().reports, 8);
    assert_eq!(manager.wait(memory, quota).unwrap(), Some(Exit::Code(42)));
    preemptions
}
